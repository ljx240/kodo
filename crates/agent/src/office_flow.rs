use crate::office::work::{self, Draft, Edit, Format, OfficeFile};
use crate::protocol::{ToolArgs, ToolCall, ToolError, ToolName, ToolResult};
use crate::{
    finish_step, run_step, AgentMode, Alive, Approve, Emit, FileDelta, RunRequest, Step, StepKind,
    StepOutcome,
};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const SHARE_PROMPT: &str = "授权发送 Office 选区";

pub fn is_path(path: &str) -> bool {
    Format::of(Path::new(path)).is_some() || crate::office::is_spreadsheet(path)
}

pub fn is_request(message: &str, attachments: &[String]) -> bool {
    let message = message.to_ascii_lowercase();
    attachments.iter().any(|path| is_path(path))
        || message
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|word| {
                matches!(
                    word,
                    "docx" | "xlsx" | "pptx" | "powerpoint" | "ppt" | "excel" | "word"
                )
            })
        || [
            "演示文稿",
            "幻灯片",
            "工作簿",
            "电子表格",
            "office 文件",
            "office文件",
        ]
        .iter()
        .any(|term| message.contains(term))
}

pub(crate) fn active(request: &RunRequest) -> bool {
    request.mode == AgentMode::Work && is_request(&request.message, &request.pinned_context)
}

fn source(project: &Path, request: &RunRequest, path: &str) -> Result<PathBuf, String> {
    if request
        .pinned_context
        .iter()
        .any(|attachment| attachment == path)
    {
        return crate::tools::resolve_pinned_file(project, path);
    }
    if request.project.is_none() {
        return Err("无项目会话只能读取本轮已选择的 Office 附件".into());
    }
    crate::tools::resolve_in_project(project, path)
}

pub struct DraftSource {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub metadata: std::fs::Metadata,
}

pub struct DraftOutput {
    pub name: String,
    pub content: Draft,
    pub source: Option<DraftSource>,
}

pub type Stage<'a> = dyn Fn(DraftOutput) -> Result<(), String> + 'a;

fn prepare_draft(
    project: &Path,
    request: &RunRequest,
    format: Format,
    from: Option<&str>,
    version: Option<&str>,
    edits: &[Edit],
    content: Option<&str>,
) -> Result<(Draft, Option<DraftSource>), String> {
    match (from, version, content) {
        (Some(from), Some(version), None) => {
            let path = source(project, request, from)?;
            let source_format = Format::of(&path).ok_or("修改需要 DOCX、XLSX 或 PPTX 原件")?;
            if source_format != format {
                return Err("修订草稿必须沿用原件格式".into());
            }
            let (bytes, metadata) = work::read_with_metadata(&path)?;
            let draft = OfficeFile::from_bytes(source_format, &bytes)?.edit(version, edits)?;
            Ok((
                draft,
                Some(DraftSource {
                    path,
                    bytes,
                    metadata,
                }),
            ))
        }
        (None, None, Some(content)) if edits.is_empty() => {
            Ok((Draft::create(format, content)?, None))
        }
        _ => Err("新建需要 content，修改需要 source、version 和 edits".into()),
    }
}

pub(crate) fn stage(
    call: &ToolCall,
    project: &Path,
    request: &RunRequest,
    stage: &Stage,
) -> ToolResult {
    let ToolArgs::OfficeWrite {
        path,
        source,
        version,
        edits,
        content,
    } = &call.args
    else {
        unreachable!("Office draft dispatch");
    };
    let prepare = || -> Result<String, String> {
        let path = Path::new(path.trim());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.starts_with('.') && !name.contains(['\\', '\n', '\r']))
            .ok_or("Office 草稿需要有效的文件名")?
            .to_owned();
        let format =
            Format::of(Path::new(&name)).ok_or("Office 草稿需要 .docx、.xlsx 或 .pptx 后缀")?;
        let (content, source) = prepare_draft(
            project,
            request,
            format,
            source.as_deref(),
            version.as_deref(),
            edits,
            content.as_deref(),
        )?;
        stage(DraftOutput {
            name: name.clone(),
            content,
            source,
        })?;
        Ok(name)
    };
    match prepare() {
        Ok(name) => ToolResult::success(
            call.id.clone(),
            call.name.label(),
            name,
            "Office 草稿可审阅，等待选择保存位置",
        ),
        Err(error) => ToolResult::failure(
            call.id.clone(),
            call.name.label(),
            path,
            ToolError::execution(error),
        ),
    }
}

pub(crate) fn execute(
    call: &ToolCall,
    project: &Path,
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
) -> StepOutcome {
    let fail =
        |error| ToolResult::failure(call.id.clone(), call.name.label(), call.args.label(), error);
    if !alive() {
        return Ok((false, None));
    }
    if !active(request) {
        return Ok((
            true,
            Some(fail(ToolError::permission_denied(
                "请在 Work 中附加或明确指定 Office 文件",
            ))),
        ));
    }
    match &call.args {
        ToolArgs::OfficeRead {
            path,
            section,
            offset,
            range,
            analysis,
        } => {
            let began = Instant::now();
            let offset = offset.unwrap_or(0);
            let prepare = || -> Result<(String, String), String> {
                let full = source(project, request, path)?;
                if let Some(analysis) = analysis {
                    if offset != 0 {
                        return Err("Excel 汇总范围使用 range，无需 offset".into());
                    }
                    let section = section.as_deref().ok_or("Excel 汇总需要指定工作表")?;
                    let range = range.as_deref().ok_or("Excel 汇总需要指定 A1 范围")?;
                    let summary = OfficeFile::open(&full)?.analyze(section, range, analysis)?;
                    let columns = summary
                        .totals
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("、");
                    let group = summary
                        .group_by
                        .as_ref()
                        .map_or_else(String::new, |column| format!("，按 {column} 列分组"));
                    let scope = format!(
                        "工作表 {}，范围 {range}，汇总 {columns} 列{group}",
                        summary.sheet
                    );
                    let data =
                        serde_json::to_string(&summary).map_err(|error| error.to_string())?;
                    if data.len() > 32_000 {
                        return Err("Excel 汇总超出结果大小限制，请减少分组或数值列".into());
                    }
                    Ok((scope, data))
                } else if Format::of(&full).is_some() {
                    let view = OfficeFile::open(&full)?.view(
                        section.as_deref(),
                        offset,
                        40,
                        range.as_deref(),
                    )?;
                    let label = view
                        .sections
                        .iter()
                        .find(|section| section.id == view.section)
                        .map(|section| section.label.as_str())
                        .unwrap_or(&view.section);
                    let items = if view.items.is_empty() {
                        "无选区内容".to_owned()
                    } else {
                        format!(
                            "第 {}–{} 项",
                            view.offset + 1,
                            view.offset + view.items.len()
                        )
                    };
                    let scope = format!(
                        "文件结构索引，{label}，{items}{}",
                        range
                            .as_ref()
                            .map_or_else(String::new, |range| format!("，范围 {range}"))
                    );
                    let data = serde_json::to_string(&view).map_err(|error| error.to_string())?;
                    Ok((scope, data))
                } else if crate::office::is_spreadsheet(path)
                    && offset == 0
                    && section.is_none()
                    && range.is_none()
                {
                    let text = crate::office::read_spreadsheet(&full, 10_000)?;
                    Ok(("旧版表格，最多 10000 字符".to_owned(), text.text))
                } else {
                    Err("该格式不支持结构化 Office 读取".into())
                }
            };
            let (scope, data) = match prepare() {
                Ok(prepared) => prepared,
                Err(error) => return Ok((true, Some(fail(ToolError::execution(error))))),
            };
            let Some(provider) = request.provider.as_ref() else {
                return Ok((
                    true,
                    Some(fail(ToolError::permission_denied("没有可用的模型接收方"))),
                ));
            };
            let endpoint = provider
                .endpoint
                .split_once("://")
                .map_or(provider.endpoint.as_str(), |(_, endpoint)| endpoint);
            let authority = endpoint.split(['/', '?', '#']).next().unwrap_or("");
            let host = authority.rsplit('@').next().unwrap_or("");
            let recipient = format!(
                "{} / {}{}",
                provider.template,
                provider.resolved_model_id(),
                if host.is_empty() {
                    String::new()
                } else {
                    format!(" ({host})")
                }
            );
            let label = format!("{SHARE_PROMPT}：{path}，{scope} → {recipient}（仅本次）");
            if !approve(StepKind::FileRead, &label) {
                return Ok((
                    true,
                    Some(fail(ToolError::permission_denied(
                        "本次 Office 内容传递未获授权",
                    ))),
                ));
            }
            let step = Step::FileRead {
                path: path.clone(),
                detail: format!(
                    "Office {}，{scope}",
                    if analysis.is_some() {
                        "数据汇总"
                    } else {
                        "结构化读取"
                    }
                ),
            };
            if !run_step(step.clone(), alive, emit) {
                return Ok((false, None));
            }
            let keep = finish_step(step, began.elapsed().as_millis() as u64, false, emit);
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::OfficeRead.label(),
                    path.clone(),
                    data,
                )),
            ))
        }
        ToolArgs::OfficeWrite {
            path,
            source: from,
            version,
            edits,
            content,
        } => {
            if request.project.is_none() {
                return Ok((
                    true,
                    Some(fail(ToolError::permission_denied(
                        "请在桌面 Office 文件入口选择保存位置",
                    ))),
                ));
            }
            let target = match crate::tools::resolve_in_project(project, path) {
                Ok(target) => target,
                Err(error) => return Ok((true, Some(fail(ToolError::path_escape(error))))),
            };
            let format = match Format::of(&target) {
                Some(format) => format,
                None => {
                    return Ok((
                        true,
                        Some(fail(ToolError::invalid_args(
                            "Office 输出需要 .docx、.xlsx 或 .pptx 后缀",
                        ))),
                    ))
                }
            };
            let began = Instant::now();
            let (draft, input) = match prepare_draft(
                project,
                request,
                format,
                from.as_deref(),
                version.as_deref(),
                edits,
                content.as_deref(),
            ) {
                Ok(prepared) => prepared,
                Err(error) => return Ok((true, Some(fail(ToolError::execution(error))))),
            };
            if draft.needs_calculation {
                return Ok((
                    true,
                    Some(fail(ToolError::execution(
                        "工作簿仍需复算，请在 Office 查看器中另存草稿",
                    ))),
                ));
            }
            let label = format!("另存 Office 文件 {path}（{} 项修改）", draft.changes.len());
            if request
                .permission
                .needs_approval(StepKind::FileChange, Some(&label))
                && !approve(StepKind::FileChange, &label)
            {
                return Ok((
                    true,
                    Some(fail(ToolError::permission_denied(
                        "用户未授权另存 Office 文件",
                    ))),
                ));
            }
            let step = Step::FileChange {
                changes: vec![FileDelta {
                    path: path.clone(),
                    added: 0,
                    removed: 0,
                }],
                file_span_ref: None,
            };
            if !run_step(step.clone(), alive, emit) {
                return Ok((false, None));
            }
            let condition = input
                .as_ref()
                .map(|source| source.path.as_path())
                .zip(version.as_deref());
            let result = match draft.save(&target, condition) {
                Ok(version) => {
                    let mut result = ToolResult::success(
                        call.id.clone(),
                        ToolName::OfficeWrite.label(),
                        path.clone(),
                        format!(
                            "已另存可编辑的 Office 文件 {path}，版本 {version}，{} 项修改",
                            draft.changes.len()
                        ),
                    );
                    result.changed_files.push(path.clone());
                    result
                }
                Err(error) => fail(ToolError::execution(error)),
            };
            let keep = finish_step(step, began.elapsed().as_millis() as u64, !result.ok, emit);
            Ok((keep, Some(result)))
        }
        _ => unreachable!("Office tool dispatch"),
    }
}
