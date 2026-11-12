#[path = "chart.rs"]
mod chart;
#[path = "create.rs"]
mod create;
#[path = "sheet.rs"]
mod sheet;
#[path = "text.rs"]
mod text;
#[path = "xml.rs"]
mod xml;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use xml::Xml;

pub use chart::{ChartKind, SlideChart};
pub use sheet::{AnalysisRequest, SheetAnalysis};

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_PART: usize = 32 * 1024 * 1024;
const MAX_PACKAGE: usize = 192 * 1024 * 1024;
const MAX_ITEMS: usize = 120;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Docx,
    Xlsx,
    Pptx,
}

impl Format {
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "docx" => Some(Self::Docx),
            "xlsx" => Some(Self::Xlsx),
            "pptx" => Some(Self::Pptx),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
            }
            Self::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
            }
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub label: String,
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub label: String,
    pub kind: &'static str,
    pub text: String,
    pub editable: bool,
    pub clipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<sheet::CellDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<chart::ChartValues>,
}

impl Item {
    fn new(
        id: String,
        label: String,
        kind: &'static str,
        mut text: String,
        editable: bool,
    ) -> Self {
        let cut = text.char_indices().nth(2_000).map(|(index, _)| index);
        if let Some(cut) = cut {
            text.truncate(cut);
        }
        Self {
            id,
            label,
            kind,
            text,
            editable: editable && cut.is_none(),
            clipped: cut.is_some(),
            cell: None,
            chart: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub format: Format,
    pub version: String,
    pub sections: Vec<Section>,
    pub section: String,
    pub items: Vec<Item>,
    pub offset: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub read_only: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    ReplaceText {
        section: String,
        item: String,
        before: String,
        after: String,
    },
    SetCell {
        section: String,
        address: String,
        value: CellValue,
    },
    SetChartData {
        section: String,
        item: String,
        categories: Vec<String>,
        values: Vec<String>,
    },
    SetNumberFormat {
        section: String,
        address: String,
        format: String,
    },
    ReplacePicture {
        section: String,
        item: String,
        data: String,
    },
    MoveShape {
        section: String,
        item: String,
        x: u32,
        y: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CellValue {
    Text(String),
    Number(String),
    Boolean(bool),
    Date(String),
    Formula(String),
    Blank,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub section: String,
    pub item: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideLayout {
    #[default]
    Body,
    Cover,
    Conclusion,
    Comparison,
    Metrics,
    Trend,
    Action,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Slide {
    pub title: String,
    #[serde(default)]
    pub layout: SlideLayout,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub secondary: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub chart: Option<SlideChart>,
    #[serde(default)]
    pub metrics: Vec<SlideMetric>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlideMetric {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub detail: String,
}

pub struct Draft {
    pub format: Format,
    pub bytes: Vec<u8>,
    pub changes: Vec<Change>,
    pub needs_calculation: bool,
}

pub struct OfficeFile {
    pub format: Format,
    pub version: String,
    package: Package,
    main: String,
    read_only: Option<String>,
}

impl OfficeFile {
    pub fn open(path: &Path) -> Result<Self, String> {
        let format = Format::of(path).ok_or("Office requires a DOCX, XLSX or PPTX file")?;
        Self::from_bytes(format, &read_bytes(path)?)
    }

    pub fn from_bytes(format: Format, bytes: &[u8]) -> Result<Self, String> {
        let package = Package::read(bytes)?;
        let main = package
            .relationships("")?
            .into_iter()
            .find(|relation| relation.kind == "officeDocument" && !relation.external)
            .ok_or("Office package has no main document")?
            .target;
        let manifest = package.xml("[Content_Types].xml")?;
        let content_type = manifest.named("Override").find_map(|index| {
            let part = &manifest.elements[index];
            (part.attr("PartName")?.strip_prefix('/')? == main)
                .then(|| part.attr("ContentType"))
                .flatten()
        });
        if content_type != Some(format.content_type()) {
            return Err("Office package type does not match its file extension".into());
        }
        let xml = package.xml(&main)?;
        let expected_root = match format {
            Format::Docx => "document",
            Format::Xlsx => "workbook",
            Format::Pptx => "presentation",
        };
        if xml.elements[0].local() != expected_root {
            return Err("Office package has an invalid main document".into());
        }
        let read_only = if package.parts.keys().any(|name| {
            let name = name.to_ascii_lowercase();
            name.starts_with("_xmlsignatures/") || name.ends_with("vbaproject.bin")
        }) {
            Some("带签名或宏的 Office 文件仅可读取".to_owned())
        } else if xml.elements[0]
            .attrs
            .iter()
            .any(|(_, value)| value.starts_with("http://purl.oclc.org/ooxml"))
        {
            Some("Strict OOXML 文件仅可读取".to_owned())
        } else {
            match format {
                Format::Docx => package
                    .related(&main, "settings")?
                    .map(|part| package.xml(&part))
                    .transpose()?
                    .and_then(|settings| {
                        settings
                            .named("documentProtection")
                            .next()
                            .map(|_| "受保护的文档仅可读取".to_owned())
                    }),
                Format::Xlsx => xml
                    .named("workbookProtection")
                    .next()
                    .map(|_| "受保护的工作簿仅可读取".to_owned()),
                Format::Pptx => xml
                    .named("modifyVerifier")
                    .next()
                    .map(|_| "受保护的演示仅可读取".to_owned()),
            }
        };
        Ok(Self {
            format,
            version: crate::read_receipt::version_of(bytes),
            package,
            main,
            read_only,
        })
    }

    pub fn preview_bytes(&self) -> Result<Vec<u8>, String> {
        self.check_renderable()?;
        self.package.write()
    }

    fn check_renderable(&self) -> Result<(), String> {
        for (name, bytes) in &self.package.parts {
            let lower = name.to_ascii_lowercase();
            if self.format == Format::Pptx
                && lower.starts_with("ppt/embeddings/")
                && lower.ends_with(".xlsx")
            {
                if bytes.len() > 2 * 1024 * 1024 {
                    return Err("图表嵌入工作簿超过预览大小上限".into());
                }
                Self::from_bytes(Format::Xlsx, bytes)?.check_renderable()?;
            } else if lower.ends_with("vbaproject.bin") || lower.contains("/embeddings/") {
                return Err("带宏或嵌入对象的文件仅提供结构预览".into());
            }
            if lower.ends_with(".rels") {
                let relations = self.package.xml(name)?;
                if relations
                    .named("Relationship")
                    .any(|index| relations.elements[index].attr("TargetMode") == Some("External"))
                {
                    return Err("带外部链接的文件仅提供结构预览".into());
                }
            }
            if lower.ends_with(".xml")
                && (std::str::from_utf8(bytes).is_err()
                    || bytes.windows(9).any(|window| window == b"<!DOCTYPE"))
            {
                return Err("含 DTD 的 Office 文件不可渲染".into());
            }
        }
        Ok(())
    }

    fn sections(&self) -> Result<Vec<Section>, String> {
        match self.format {
            Format::Docx => text::word_sections(&self.package, &self.main),
            Format::Xlsx => sheet::sections(&self.package, &self.main),
            Format::Pptx => text::presentation_sections(&self.package, &self.main),
        }
    }

    pub fn analyze(
        &self,
        section: &str,
        range: &str,
        request: &AnalysisRequest,
    ) -> Result<SheetAnalysis, String> {
        if self.format != Format::Xlsx {
            return Err("数据汇总需要 XLSX 工作簿".into());
        }
        let sections = self.sections()?;
        let selected = sections
            .iter()
            .find(|item| item.id == section || item.label == section)
            .ok_or("Excel 工作表未找到")?;
        sheet::analyze(
            &self.package,
            &self.main,
            selected,
            &self.version,
            range,
            request,
        )
    }

    pub fn view(
        &self,
        section: Option<&str>,
        offset: usize,
        limit: usize,
        range: Option<&str>,
    ) -> Result<View, String> {
        let sections = self.sections()?;
        let selected = match section {
            Some(id) => sections
                .iter()
                .find(|section| section.id == id || section.label == id),
            None => sections.first(),
        }
        .ok_or("Office section was not found")?;
        let section = selected.id.clone();
        let limit = limit.clamp(1, MAX_ITEMS);
        let (total, mut items) = match self.format {
            Format::Docx | Format::Pptx => text::items(&self.package, selected, offset, limit)?,
            Format::Xlsx => {
                sheet::items(&self.package, &self.main, selected, offset, limit, range)?
            }
        };
        let mut remaining = 32_000;
        let count = items
            .iter()
            .take_while(|item| {
                let cost = item.text.len()
                    + item.cell.as_ref().map_or(0, |cell| cell.byte_len())
                    + item.chart.as_ref().map_or(0, |chart| chart.byte_len());
                let fits = cost <= remaining;
                remaining = remaining.saturating_sub(cost);
                fits
            })
            .count();
        items.truncate(count);
        if self.read_only.is_some() {
            for item in &mut items {
                item.editable = false;
            }
        }
        let next = offset.saturating_add(items.len());
        Ok(View {
            format: self.format,
            version: self.version.clone(),
            sections,
            section,
            items,
            offset,
            total,
            next_offset: (next < total).then_some(next),
            read_only: self.read_only.clone(),
        })
    }

    pub fn edit(mut self, version: &str, operations: &[Edit]) -> Result<Draft, String> {
        if self.version != version {
            return Err("stale_source: 文件已更改，请重新读取选区".into());
        }
        if let Some(reason) = &self.read_only {
            return Err(reason.clone());
        }
        if operations.is_empty() || operations.len() > 100 {
            return Err("Office 编辑需要 1 至 100 项操作".into());
        }
        let sections = self.sections()?;
        let mut changes = Vec::with_capacity(operations.len());
        let mut calculation_changes = Vec::new();
        for operation in operations {
            let id = match operation {
                Edit::ReplaceText { section, .. }
                | Edit::SetCell { section, .. }
                | Edit::SetChartData { section, .. }
                | Edit::SetNumberFormat { section, .. }
                | Edit::MoveShape { section, .. }
                | Edit::ReplacePicture { section, .. } => section,
            };
            let section = sections
                .iter()
                .find(|section| &section.id == id || &section.label == id)
                .ok_or("Office edit section was not found")?;
            let change = match (self.format, operation) {
                (
                    Format::Docx | Format::Pptx,
                    Edit::ReplaceText {
                        item,
                        before,
                        after,
                        ..
                    },
                ) => text::replace(&mut self.package, section, item, before, after)?,
                (Format::Xlsx, Edit::SetCell { address, value, .. }) => {
                    let change = sheet::set_cell(&mut self.package, section, address, value)?;
                    calculation_changes.push(change.clone());
                    change
                }
                (
                    Format::Pptx,
                    Edit::SetChartData {
                        item,
                        categories,
                        values,
                        ..
                    },
                ) => chart::update_presentation(
                    &mut self.package,
                    section,
                    item,
                    categories,
                    values,
                )?,
                (
                    Format::Xlsx,
                    Edit::SetNumberFormat {
                        address, format, ..
                    },
                ) => sheet::set_number_format(
                    &mut self.package,
                    &self.main,
                    section,
                    address,
                    format,
                )?,
                (Format::Pptx, Edit::MoveShape { item, x, y, .. }) => {
                    text::move_shape(&mut self.package, &self.main, section, item, *x, *y)?
                }
                (Format::Pptx, Edit::ReplacePicture { item, data, .. }) => {
                    text::replace_picture(&mut self.package, section, item, data)?
                }
                _ => return Err("操作与 Office 文件格式不匹配".into()),
            };
            changes.push(change);
        }
        let needs_calculation = !calculation_changes.is_empty()
            && sheet::refresh_calculation(
                &mut self.package,
                &self.main,
                &sections,
                &calculation_changes,
            )?;
        let bytes = self.package.write()?;
        Ok(Draft {
            format: self.format,
            bytes,
            changes,
            needs_calculation,
        })
    }
}

impl Draft {
    pub fn create(format: Format, content: &str) -> Result<Self, String> {
        xml::validate_text(content)?;
        let (package, needs_calculation) = create::build(format, content)?;
        let bytes = package.write()?;
        OfficeFile::from_bytes(format, &bytes)?.view(None, 0, 1, None)?;
        Ok(Self {
            format,
            bytes,
            changes: Vec::new(),
            needs_calculation,
        })
    }

    /// Only publishes a new file. The staged inode becomes the destination
    /// atomically and an existing destination always wins.
    pub fn save(&self, target: &Path, source: Option<(&Path, &str)>) -> Result<String, String> {
        let version = self.save_checked(target, |_| {
            if let Some((source, version)) = source {
                if crate::read_receipt::version_of(&read_bytes(source)?) != version {
                    return Err("stale_source: 原件已更改，请重新读取后另存".into());
                }
            }
            Ok(())
        })?;
        sync_directory(target.parent().unwrap())
            .map_err(|error| format!("Office 文件已另存，目录持久化失败：{error}"))?;
        Ok(version)
    }

    pub fn save_checked(
        &self,
        target: &Path,
        before_publish: impl FnOnce(&fs::Metadata) -> Result<(), String>,
    ) -> Result<String, String> {
        if Format::of(target) != Some(self.format) {
            return Err(format!("另存文件必须以 .{} 结尾", self.format.extension()));
        }
        let parent = target.parent().ok_or("保存位置没有父目录")?;
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let stage = parent.join(format!(".kodo-office-{}-{id}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&stage).map_err(|error| error.to_string())?;
        let result = (|| {
            file.write_all(&self.bytes)
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            before_publish(&file.metadata().map_err(|error| error.to_string())?)?;
            fs::hard_link(&stage, target).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    "目标文件已存在，请选择新文件名".to_owned()
                } else {
                    format!("无法排他保存 Office 文件：{error}")
                }
            })?;
            Ok(crate::read_receipt::version_of(&self.bytes))
        })();
        drop(file);
        let _ = fs::remove_file(&stage);
        result
    }
}

pub fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    read_with_metadata(path).map(|(bytes, _)| bytes)
}

pub fn read_with_metadata(path: &Path) -> Result<(Vec<u8>, fs::Metadata), String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Office 文件必须是普通文件".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err("Office 文件超过 64 MiB".into());
    }
    Ok((bytes, metadata))
}

struct Relationship {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

#[derive(Default)]
struct Package {
    parts: BTreeMap<String, Vec<u8>>,
}

impl Package {
    fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_FILE {
            return Err("Office 文件超过 64 MiB".into());
        }
        if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]) {
            return Err("无法读取加密或旧版二进制 Office 文件".into());
        }
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
        if archive.len() > 4096 {
            return Err("Office 压缩包条目数超过上限".into());
        }
        let mut package = Self::default();
        let mut total = 0;
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
            let name = file.name().to_owned();
            if file.encrypted() || file.is_symlink() || !valid_part(name.trim_end_matches('/')) {
                return Err("Office 压缩包含有加密、链接或越界条目".into());
            }
            if file.is_dir() {
                continue;
            }
            if file.size() > MAX_PART as u64 || total + file.size() > MAX_PACKAGE as u64 {
                return Err("Office 压缩包解压体积超过上限".into());
            }
            let mut data = Vec::new();
            file.by_ref()
                .take(MAX_PART as u64 + 1)
                .read_to_end(&mut data)
                .map_err(|error| error.to_string())?;
            total += data.len() as u64;
            if data.len() as u64 != file.size() || total > MAX_PACKAGE as u64 {
                return Err("Office 条目解压大小与声明不符".into());
            }
            if package.parts.insert(name, data).is_some() {
                return Err("Office 压缩包含有重复条目".into());
            }
        }
        Ok(package)
    }

    fn xml(&self, part: &str) -> Result<Xml<'_>, String> {
        let bytes = self
            .parts
            .get(part)
            .ok_or_else(|| format!("Office 缺少部件 {part}"))?;
        let xml = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
        Xml::parse(xml)
    }

    fn set(&mut self, part: impl Into<String>, content: impl Into<Vec<u8>>) {
        self.parts.insert(part.into(), content.into());
    }

    fn write(&self) -> Result<Vec<u8>, String> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (part, bytes) in &self.parts {
            writer
                .start_file(part, options)
                .map_err(|error| error.to_string())?;
            writer.write_all(bytes).map_err(|error| error.to_string())?;
        }
        let bytes = writer
            .finish()
            .map_err(|error| error.to_string())?
            .into_inner();
        if bytes.len() > MAX_FILE {
            return Err("Office 成果超过 64 MiB".into());
        }
        Ok(bytes)
    }

    fn relationships(&self, owner: &str) -> Result<Vec<Relationship>, String> {
        let (directory, leaf) = owner.rsplit_once('/').unwrap_or(("", owner));
        let rels = if owner.is_empty() {
            "_rels/.rels".to_owned()
        } else if directory.is_empty() {
            format!("_rels/{leaf}.rels")
        } else {
            format!("{directory}/_rels/{leaf}.rels")
        };
        if !self.parts.contains_key(&rels) {
            return Ok(Vec::new());
        }
        let xml = self.xml(&rels)?;
        let mut relationships = Vec::new();
        for index in xml.named("Relationship") {
            let node = &xml.elements[index];
            let kind = node.attr("Type").ok_or("Office 关系缺少类型")?;
            let target = node.attr("Target").ok_or("Office 关系缺少目标")?;
            let external = node.attr("TargetMode") == Some("External");
            relationships.push(Relationship {
                id: node.attr("Id").ok_or("Office 关系缺少标识")?.to_owned(),
                kind: kind.rsplit('/').next().unwrap().to_owned(),
                target: if external {
                    target.to_owned()
                } else {
                    resolve_part(directory, target)?
                },
                external,
            });
        }
        Ok(relationships)
    }

    fn related(&self, owner: &str, kind: &str) -> Result<Option<String>, String> {
        Ok(self
            .relationships(owner)?
            .into_iter()
            .find(|relation| relation.kind == kind && !relation.external)
            .map(|relation| relation.target))
    }
}

fn valid_part(part: &str) -> bool {
    !part.contains(['\\', ':', '\0'])
        && part
            .split('/')
            .all(|component| !matches!(component, "" | "." | ".."))
}

fn resolve_part(directory: &str, target: &str) -> Result<String, String> {
    let mut parts: Vec<&str> = if target.starts_with('/') || directory.is_empty() {
        Vec::new()
    } else {
        directory.split('/').collect()
    };
    for component in target.trim_start_matches('/').split('/') {
        match component {
            "." => {}
            ".." if parts.pop().is_some() => {}
            ".." => return Err("Office 部件关系越出包范围".into()),
            component => parts.push(component),
        }
    }
    let result = parts.join("/");
    if !valid_part(&result) {
        return Err("Office 部件关系含无效路径".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_edit_across_runs_keeps_styles_and_unrelated_parts() {
        let original = Draft::create(Format::Docx, "# 月报\n已完成 8 项").unwrap();
        let mut package = Package::read(&original.bytes).unwrap();
        let part = "word/document.xml";
        let document = std::str::from_utf8(&package.parts[part]).unwrap().replace(
            r#"<w:t xml:space="preserve">已完成 8 项</w:t>"#,
            r#"<w:t xml:space="preserve">已完成 </w:t></w:r><w:r><w:t>8 项</w:t>"#,
        );
        package.set(part, document);
        let bytes = package.write().unwrap();
        let file = OfficeFile::from_bytes(Format::Docx, &bytes).unwrap();
        let version = file.version.clone();
        let draft = file
            .edit(
                &version,
                &[Edit::ReplaceText {
                    section: part.to_owned(),
                    item: "p:2".to_owned(),
                    before: "已完成 8 项".to_owned(),
                    after: "已完成 9 项".to_owned(),
                }],
            )
            .unwrap();
        let edited = OfficeFile::from_bytes(Format::Docx, &draft.bytes).unwrap();
        assert_eq!(
            edited.view(None, 0, 20, None).unwrap().items[1].text,
            "已完成 9 项"
        );
        for (name, data) in &package.parts {
            if name != part {
                assert_eq!(&edited.package.parts[name], data, "{name}");
            }
        }
    }

    #[test]
    fn workbook_updates_typed_cell_and_cross_sheet_sum() {
        let original = Draft::create(Format::Xlsx, r#"{"sheets":[
            {"name":"数据","rows":[[{"type":"text","value":"编号"},{"type":"text","value":"金额"}],[{"type":"text","value":"0007"},{"type":"number","value":"100"}],[{"type":"blank"},{"type":"number","value":"200"}],[{"type":"blank"},{"type":"number","value":"300"}]]},
            {"name":"汇总","rows":[[{"type":"text","value":"统计"}],[{"type":"formula","value":"SUM(数据!B2:B4)"}]]}
        ]}"#).unwrap();
        let file = OfficeFile::from_bytes(Format::Xlsx, &original.bytes).unwrap();
        let version = file.version.clone();
        let draft = file
            .edit(
                &version,
                &[Edit::SetCell {
                    section: "数据".to_owned(),
                    address: "B3".to_owned(),
                    value: CellValue::Number("250".to_owned()),
                }],
            )
            .unwrap();
        assert!(!draft.needs_calculation);
        let edited = OfficeFile::from_bytes(Format::Xlsx, &draft.bytes).unwrap();
        let summary = edited.view(Some("汇总"), 0, 20, Some("A2")).unwrap();
        assert_eq!(summary.items[0].text, "650");
        assert_eq!(
            summary.items[0].cell.as_ref().unwrap().formula.as_deref(),
            Some("SUM(数据!B2:B4)")
        );
        assert_eq!(
            edited.view(Some("数据"), 0, 20, Some("A2")).unwrap().items[0].text,
            "0007"
        );
    }

    #[test]
    fn presentation_edits_notes_without_rebuilding_the_slide() {
        let original = Draft::create(Format::Pptx, r#"{"slides":[{"title":"季度汇报","body":["增长率"],"notes":"旧备注"},{"title":"下一步"}]}"#).unwrap();
        let file = OfficeFile::from_bytes(Format::Pptx, &original.bytes).unwrap();
        let original_slide = file.package.parts["ppt/slides/slide1.xml"].clone();
        let version = file.version.clone();
        let draft = file
            .edit(
                &version,
                &[Edit::ReplaceText {
                    section: "第 1 页备注".into(),
                    item: "p:1".into(),
                    before: "旧备注".into(),
                    after: "新备注".into(),
                }],
            )
            .unwrap();
        let edited = OfficeFile::from_bytes(Format::Pptx, &draft.bytes).unwrap();
        assert_eq!(
            edited.package.parts["ppt/slides/slide1.xml"],
            original_slide
        );
        assert_eq!(
            edited.view(Some("第 1 页备注"), 0, 10, None).unwrap().items[0].text,
            "新备注"
        );
    }

    #[test]
    fn office_copy_refuses_existing_target_and_stale_source() {
        let scratch = std::env::temp_dir().join(format!(
            "kodo-office-test-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&scratch).unwrap();
        let source = scratch.join("source.docx");
        let target = scratch.join("copy.docx");
        let draft = Draft::create(Format::Docx, "# 月报").unwrap();
        let version = draft.save(&source, None).unwrap();
        fs::write(&target, b"existing").unwrap();
        assert!(draft.save(&target, Some((&source, &version))).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"existing");
        let after = scratch.join("after.docx");
        fs::write(&source, b"changed externally").unwrap();
        assert!(draft
            .save(&after, Some((&source, &version)))
            .unwrap_err()
            .contains("stale_source"));
        assert!(!after.exists());
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn worksheet_analysis_groups_numeric_values_and_marks_formula_caches() {
        let workbook = Draft::create(
            Format::Xlsx,
            r#"{"sheets":[{"name":"采购","rows":[
                ["状态","金额"],
                ["已入库",100],
                ["已入库",{"type":"formula","value":"B2*2"}],
                ["待入库",40],
                ["待入库","200"],
                ["待入库",null]
            ]}]}"#,
        )
        .unwrap();
        let file = OfficeFile::from_bytes(Format::Xlsx, &workbook.bytes).unwrap();
        let request = AnalysisRequest {
            columns: vec!["B".into()],
            group_by: Some("A".into()),
            header: true,
        };
        let analysis = file.analyze("采购", "A1:B6", &request).unwrap();
        let result = serde_json::to_value(analysis).unwrap();
        assert_eq!(result["version"], file.version);
        assert_eq!(result["rows"], 5);
        assert_eq!(result["totals"]["B"]["count"], 3);
        assert_eq!(result["totals"]["B"]["sum"], 340.0);
        assert_eq!(result["totals"]["B"]["cachedFormula"], 1);
        assert_eq!(result["totals"]["B"]["nonNumeric"], 1);
        assert_eq!(result["totals"]["B"]["blank"], 1);
        assert_eq!(result["groups"][0]["group"]["value"], "已入库");
        assert_eq!(result["groups"][0]["columns"]["B"]["average"], 150.0);
        assert_eq!(result["groups"][1]["columns"]["B"]["sum"], 40.0);
        assert!(file.analyze("采购", "A1:A6", &request).is_err());
    }

    #[test]
    fn erp_workbook_recalculates_movements_amounts_and_chart() {
        let input = serde_json::json!({"sheets":[
            {"name":"采购明细","rows":[
                [{"type":"text","value":"编码"},{"type":"text","value":"数量"},{"type":"text","value":"单价"},{"type":"text","value":"金额"},{"type":"text","value":"状态"}],
                [{"type":"text","value":"M-001"},{"type":"number","value":"30"},{"type":"number","value":"118"},{"type":"formula","value":"B2*C2"},{"type":"text","value":"已入库"}],
                [{"type":"text","value":"M-001"},{"type":"number","value":"20"},{"type":"number","value":"120"},{"type":"formula","value":"B3*C3"},{"type":"text","value":"待入库"}]
            ]},
            {"name":"库存台账","rows":[
                [{"type":"text","value":"编码"},{"type":"text","value":"入库"},{"type":"text","value":"库存"}],
                [{"type":"text","value":"M-001"},{"type":"formula","value":"SUMIFS('采购明细'!$B$2:$B$3,'采购明细'!$A$2:$A$3,A2,'采购明细'!$E$2:$E$3,\"已入库\")"},{"type":"formula","value":"80+B2-65"}]
            ],"chart":{"kind":"bar","title":"库存（件）","category_column":1,"value_column":3}},
            {"name":"总览","rows":[
                [{"type":"text","value":"采购金额"},{"type":"text","value":"库存"}],
                [{"type":"formula","value":"SUMIFS('采购明细'!D2:D3,'采购明细'!E2:E3,\"已入库\")"},{"type":"formula","value":"SUM('库存台账'!C2:C2)"}]
            ]}
        ]}).to_string();
        let draft = Draft::create(Format::Xlsx, &input).unwrap();
        assert!(!draft.needs_calculation);
        let original = OfficeFile::from_bytes(Format::Xlsx, &draft.bytes).unwrap();
        let view = original.view(Some("总览"), 0, 2, Some("A2:B2")).unwrap();
        assert_eq!(
            view.items
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>(),
            ["3540", "45"]
        );
        let edited = original
            .edit(
                &view.version,
                &[Edit::SetCell {
                    section: "采购明细".into(),
                    address: "B2".into(),
                    value: CellValue::Number("40".into()),
                }],
            )
            .unwrap();
        assert!(!edited.needs_calculation);
        let book = OfficeFile::from_bytes(Format::Xlsx, &edited.bytes).unwrap();
        let view = book.view(Some("总览"), 0, 2, Some("A2:B2")).unwrap();
        assert_eq!(
            view.items
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>(),
            ["4720", "55"]
        );
        let chart = book.package.xml("xl/charts/chart2.xml").unwrap();
        let index = chart.named("numCache").next().unwrap();
        assert!(chart.source[chart.elements[index].inner.clone()].contains(">55<"));
    }

    #[test]
    fn native_excel_chart_follows_changed_values_and_styles_are_local() {
        let original = Draft::create(Format::Xlsx, r#"{"sheets":[{
            "name":"数据", "chart":{"kind":"column","title":"地区收入","category_column":1,"value_column":2},
            "rows":[[{"type":"text","value":"区域"},{"type":"text","value":"金额"}],
                [{"type":"text","value":"华北"},{"type":"number","value":"100"}],
                [{"type":"text","value":"华东"},{"type":"number","value":"200"}]]
        }]}"#).unwrap();
        let file = OfficeFile::from_bytes(Format::Xlsx, &original.bytes).unwrap();
        let version = file.version.clone();
        let edited = file
            .edit(
                &version,
                &[
                    Edit::SetCell {
                        section: "数据".into(),
                        address: "B3".into(),
                        value: CellValue::Number("250".into()),
                    },
                    Edit::SetNumberFormat {
                        section: "数据".into(),
                        address: "B3".into(),
                        format: "0.00".into(),
                    },
                ],
            )
            .unwrap();
        assert!(!edited.needs_calculation);
        let file = OfficeFile::from_bytes(Format::Xlsx, &edited.bytes).unwrap();
        let view = file.view(None, 0, 20, Some("B2:B3")).unwrap();
        assert!(view.items[0].cell.as_ref().unwrap().number_format.is_none());
        assert_eq!(
            view.items[1]
                .cell
                .as_ref()
                .unwrap()
                .number_format
                .as_deref(),
            Some("0.00")
        );
        let chart = file.package.xml("xl/charts/chart1.xml").unwrap();
        let value = chart.named("numCache").next().unwrap();
        assert!(chart.source[chart.elements[value].inner.clone()].contains(">250<"));
    }

    #[test]
    fn presentation_picture_update_only_changes_its_media_part() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let original = Draft::create(Format::Pptx, r#"{"slides":[{"title":"月报"}]}"#).unwrap();
        let mut package = Package::read(&original.bytes).unwrap();
        let part = "ppt/slides/slide1.xml";
        let slide = String::from_utf8(package.parts[part].clone()).unwrap();
        let picture = r#"<p:pic><p:nvPicPr><p:cNvPr id="42" name="月报插图"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId42"/><a:srcRect l="2000"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="1000" y="2000"/><a:ext cx="1200000" cy="800000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#;
        package.set(
            part,
            slide.replace("</p:spTree>", &format!("{picture}</p:spTree>")),
        );
        let rels = "ppt/slides/_rels/slide1.xml.rels";
        let links = String::from_utf8(package.parts[rels].clone()).unwrap();
        let link = r#"<Relationship Id="rId42" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>"#;
        package.set(
            rels,
            links.replace("</Relationships>", &format!("{link}</Relationships>")),
        );
        let media = "ppt/media/image1.png";
        let source = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jYHoAAAAASUVORK5CYII=").unwrap();
        package.set(media, source);
        let new_image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let file = OfficeFile::from_bytes(Format::Pptx, &package.write().unwrap()).unwrap();
        let version = file.version.clone();
        let draft = file
            .edit(
                &version,
                &[Edit::ReplacePicture {
                    section: part.into(),
                    item: "picture:42".into(),
                    data: new_image.into(),
                }],
            )
            .unwrap();
        let result = Package::read(&draft.bytes).unwrap();
        assert_eq!(result.parts[media], STANDARD.decode(new_image).unwrap());
        for (name, data) in &package.parts {
            if name != media {
                assert_eq!(&result.parts[name], data, "{name}");
            }
        }
    }

    #[test]
    fn native_presentation_chart_edit_updates_embedded_workbook() {
        let original = Draft::create(
            Format::Pptx,
            r#"{"slides":[{"title":"月报","chart":{
            "kind":"bar","title":"区域","categories":["华北","华东"],"values":["100","200"]}}]}"#,
        )
        .unwrap();
        let file = OfficeFile::from_bytes(Format::Pptx, &original.bytes).unwrap();
        let view = file.view(None, 0, 20, None).unwrap();
        let chart = view.items.iter().find(|item| item.kind == "chart").unwrap();
        let edited = file
            .edit(
                &view.version,
                &[Edit::SetChartData {
                    section: view.section,
                    item: chart.id.clone(),
                    categories: vec!["华北".into(), "华南".into()],
                    values: vec!["100".into(), "250".into()],
                }],
            )
            .unwrap();
        let result = OfficeFile::from_bytes(Format::Pptx, &edited.bytes).unwrap();
        let view = result.view(None, 0, 20, None).unwrap();
        let chart = view
            .items
            .iter()
            .find_map(|item| item.chart.as_ref())
            .unwrap();
        assert_eq!(chart.categories, ["华北", "华南"]);
        assert_eq!(chart.values, ["100", "250"]);
        let embedded = OfficeFile::from_bytes(
            Format::Xlsx,
            &result.package.parts["ppt/embeddings/data1.xlsx"],
        )
        .unwrap();
        assert_eq!(
            embedded.view(None, 0, 1, Some("B3")).unwrap().items[0].text,
            "250"
        );
        result.preview_bytes().unwrap();
    }
}
