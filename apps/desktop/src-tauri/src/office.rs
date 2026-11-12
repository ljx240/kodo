use crate::apps::AppService;
use kodo_agent::office::work::{self, Change, Draft, Format, OfficeFile, View};
use kodo_agent::office_flow::DraftOutput;
use kodo_agent::process::{EnvPolicy, ProcessRunner, ProcessSpec};
use kodo_core::apps::{ArtifactDraft, ArtifactKind};
use kodo_sandbox::{SandboxContext, SandboxPolicy, SandboxRuntime};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Default)]
pub struct OfficeService {
    sources: Mutex<HashMap<String, Source>>,
    attachments: Mutex<HashSet<PathBuf>>,
    drafts: Mutex<HashMap<String, Arc<PendingDraft>>>,
}

#[derive(Clone)]
struct Source {
    path: PathBuf,
    sha: [u8; 32],
    identity: (u64, u64),
    preview: PreviewCache,
}

struct PendingDraft {
    content: Draft,
    source: Option<Source>,
    name: String,
    session: String,
    turn: u32,
    preview: PreviewCache,
    published: Mutex<Option<Published>>,
}

#[derive(Clone)]
struct Published {
    path: PathBuf,
    result_id: Option<String>,
    draft_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedOffice {
    handle: String,
    name: String,
    source_path: String,
    view: View,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeDraftView {
    handle: String,
    name: String,
    changes: Vec<Change>,
    format: Format,
    needs_calculation: bool,
    needs_preview: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeSaved {
    path: String,
    result_id: Option<String>,
    registration_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficePreview {
    image: String,
    pages: usize,
    version: String,
}

impl OfficeService {
    pub(crate) fn grant_attachment(&self, path: &Path) {
        if Format::of(path).is_some() {
            if let Ok(file) =
                kodo_agent::tools::resolve_pinned_file(Path::new(""), &path.to_string_lossy())
            {
                self.attachments.lock().insert(file);
            }
        }
    }

    fn open(&self, path: PathBuf) -> Result<OpenedOffice, String> {
        let format = Format::of(&path).ok_or("请选择 DOCX、XLSX 或 PPTX 文件")?;
        let (bytes, metadata) = work::read_with_metadata(&path)?;
        let view = OfficeFile::from_bytes(format, &bytes)?.view(None, 0, 60, None)?;
        let handle = uuid::Uuid::new_v4().to_string();
        let name = path
            .file_name()
            .ok_or("Office 文件缺少文件名")?
            .to_string_lossy()
            .into_owned();
        let mut sources = self.sources.lock();
        if sources.len() >= 12 {
            return Err("打开的 Office 文件过多，请先关闭其他预览".into());
        }
        let source_path = path.to_string_lossy().into_owned();
        sources.insert(
            handle.clone(),
            Source {
                path,
                sha: Sha256::digest(&bytes).into(),
                identity: identity(&metadata)?,
                preview: PreviewCache::default(),
            },
        );
        Ok(OpenedOffice {
            handle,
            name,
            source_path,
            view,
        })
    }

    fn source(&self, handle: &str) -> Result<Source, String> {
        self.sources
            .lock()
            .get(handle)
            .cloned()
            .ok_or_else(|| "Office 文件句柄已关闭".to_owned())
    }

    fn draft(&self, handle: &str) -> Result<Arc<PendingDraft>, String> {
        self.drafts
            .lock()
            .get(handle)
            .cloned()
            .ok_or_else(|| "Office 草稿已关闭".to_owned())
    }

    fn stage(
        &self,
        content: Draft,
        source: Option<Source>,
        session: String,
        turn: u32,
        name: String,
    ) -> Result<(), String> {
        let handle = uuid::Uuid::new_v4().to_string();
        let mut drafts = self.drafts.lock();
        if drafts.len() >= 12 {
            return Err("Office 草稿过多，请先关闭其他草稿".into());
        }
        let pending = PendingDraft {
            content,
            source,
            name,
            session,
            turn,
            preview: PreviewCache::default(),
            published: Mutex::new(None),
        };
        drafts.insert(handle, Arc::new(pending));
        Ok(())
    }
}

impl PendingDraft {
    fn requires_preview(&self) -> bool {
        self.source.is_none() || self.content.format != Format::Xlsx
    }

    fn view(&self, handle: &str) -> OfficeDraftView {
        OfficeDraftView {
            handle: handle.to_owned(),
            name: self.name.clone(),
            changes: self.content.changes.clone(),
            format: self.content.format,
            needs_calculation: self.content.needs_calculation,
            needs_preview: self.requires_preview(),
        }
    }
}

pub fn stage_for_turn(
    app: &AppHandle,
    session: &str,
    turn: u32,
    draft: DraftOutput,
) -> Result<(), String> {
    let source = draft
        .source
        .map(|source| {
            Ok::<_, String>(Source {
                path: source.path,
                sha: Sha256::digest(&source.bytes).into(),
                identity: identity(&source.metadata)?,
                preview: PreviewCache::default(),
            })
        })
        .transpose()?;
    app.state::<OfficeService>()
        .stage(draft.content, source, session.to_owned(), turn, draft.name)
}

#[tauri::command]
pub fn office_pending_drafts(
    state: State<'_, OfficeService>,
    session: String,
    turn: u32,
) -> Vec<OfficeDraftView> {
    let mut drafts = state
        .drafts
        .lock()
        .iter()
        .filter(|(_, draft)| draft.session == session && draft.turn == turn)
        .map(|(handle, draft)| draft.view(handle))
        .collect::<Vec<_>>();
    drafts.sort_by(|a, b| a.name.cmp(&b.name));
    drafts
}

impl Source {
    fn bytes(&self) -> Result<Vec<u8>, String> {
        let path =
            kodo_agent::tools::resolve_pinned_file(Path::new(""), &self.path.to_string_lossy())?;
        let (bytes, metadata) = work::read_with_metadata(&path)?;
        let sha: [u8; 32] = Sha256::digest(&bytes).into();
        if path != self.path || sha != self.sha || identity(&metadata)? != self.identity {
            return Err("stale_source: 原件已更改，请关闭预览并重新打开".into());
        }
        Ok(bytes)
    }

    fn file(&self) -> Result<OfficeFile, String> {
        OfficeFile::from_bytes(
            Format::of(&self.path).ok_or("Office 文件格式无效")?,
            &self.bytes()?,
        )
    }
}

#[tauri::command]
pub async fn office_pick(app: AppHandle) -> Result<Option<OpenedOffice>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("打开 Office 文件")
        .add_filter("Office", &["docx", "xlsx", "pptx"])
        .blocking_pick_file();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|error| error.to_string())?;
    let full = kodo_agent::tools::resolve_pinned_file(Path::new(""), &path.to_string_lossy())?;
    app.state::<OfficeService>().open(full).map(Some)
}

#[tauri::command]
pub fn office_open_project(
    state: State<'_, OfficeService>,
    project: String,
    path: String,
) -> Result<OpenedOffice, String> {
    let root = Path::new(&project)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let projects = kodo_core::workspace::load(&crate::log()?).map_err(|error| error.to_string())?;
    if !projects.iter().any(|project| project.path == root) {
        return Err("Office 文件不在已授权项目中".into());
    }
    let file = kodo_agent::tools::resolve_in_project(&root, &path)?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    state.open(file)
}

#[tauri::command]
pub fn office_open_result(
    state: State<'_, OfficeService>,
    results: State<'_, AppService>,
    id: String,
) -> Result<OpenedOffice, String> {
    let path = results
        .store()?
        .artifact_file_path(&id)
        .map_err(|error| error.code().to_owned())?;
    let full = kodo_agent::tools::resolve_pinned_file(Path::new(""), &path)?;
    state.open(full)
}

#[tauri::command]
pub fn office_open_attachment(
    state: State<'_, OfficeService>,
    path: String,
    session: Option<String>,
) -> Result<OpenedOffice, String> {
    if !Path::new(&path).is_absolute() {
        return Err("Office 附件需要绝对路径".into());
    }
    let file = kodo_agent::tools::resolve_pinned_file(Path::new(""), &path)?;
    let granted = state.attachments.lock().contains(&file);
    if !granted {
        let session = session.ok_or("请通过系统文件选择器添加 Office 附件")?;
        let session = kodo_core::session::load(&crate::sessions()?, &session)
            .map_err(|error| error.to_string())?;
        if !session
            .turns
            .iter()
            .any(|turn| turn.context.contains(&path))
        {
            return Err("该文件未添加到此会话".into());
        }
    }
    state.open(file)
}

#[tauri::command]
pub fn office_view(
    state: State<'_, OfficeService>,
    handle: String,
    draft: bool,
    section: Option<String>,
    offset: usize,
    range: Option<String>,
) -> Result<View, String> {
    let file = if draft {
        let pending = state.draft(&handle)?;
        OfficeFile::from_bytes(pending.content.format, &pending.content.bytes)?
    } else {
        state.source(&handle)?.file()?
    };
    file.view(section.as_deref(), offset, 60, range.as_deref())
}

#[tauri::command]
pub async fn office_save(
    app: AppHandle,
    handle: String,
    save_as_draft: bool,
) -> Result<Option<OfficeSaved>, String> {
    let draft = app.state::<OfficeService>().draft(&handle)?;
    let needs_picker = draft.published.lock().is_none();
    let target = if needs_picker {
        let picked = app
            .dialog()
            .file()
            .set_title("另存 Office 文件")
            .set_file_name(&draft.name)
            .add_filter("Office", &[draft.content.format.extension()])
            .blocking_save_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|error| error.to_string())?;
        let parent = path
            .parent()
            .ok_or("保存位置没有父目录")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let target = parent.join(path.file_name().ok_or("保存位置没有文件名")?);
        if kodo_agent::tools::is_sensitive_relative_path(&target.to_string_lossy()) {
            return Err("无法在受保护目录保存 Office 文件".into());
        }
        Some(target)
    } else {
        None
    };

    let mut publication = draft.published.lock();
    if publication.is_none() {
        let target = target.ok_or("未选择 Office 文件保存位置")?;
        if !save_as_draft {
            if draft.content.needs_calculation {
                return Err("该工作簿仍需复算，请按草稿保存".into());
            }
            if draft.requires_preview()
                && !draft
                    .preview
                    .lock()
                    .as_ref()
                    .is_some_and(|document| document.previewed)
            {
                return Err("请先查看草稿预览，或按草稿保存".into());
            }
        }
        let mut journal = None;
        let save = draft.content.save_checked(&target, |metadata| {
            if !save_as_draft {
                let record = Publication::new(&target, &draft.content.bytes, metadata)?;
                journal = Some(write_publication(&handle, &record)?);
            }
            if let Some(source) = &draft.source {
                source.bytes()?;
            }
            Ok(())
        });
        if let Err(error) = save {
            if let Some(path) = journal {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        *publication = Some(Published {
            path: target,
            result_id: None,
            draft_only: save_as_draft,
        });
    }

    let publication = publication.as_mut().unwrap();
    let path = publication.path.to_string_lossy().into_owned();
    if let Some(id) = &publication.result_id {
        return Ok(Some(OfficeSaved {
            path,
            result_id: Some(id.clone()),
            registration_error: None,
        }));
    }
    let synced = work::sync_directory(publication.path.parent().unwrap());
    if publication.draft_only {
        return Ok(Some(OfficeSaved {
            path,
            result_id: None,
            registration_error: synced.err(),
        }));
    }
    let registration = synced
        .and_then(|_| read_publication(&handle))
        .and_then(|record| {
            if !record.matches()? {
                return Err("已保存文件的身份或内容已变化".into());
            }
            register_result(&app, &path, &handle)
        });
    let (result_id, registration_error) = match registration {
        Ok(id) => {
            publication.result_id = Some(id.clone());
            if let Ok(journal) = publication_path(&handle) {
                let _ = fs::remove_file(journal);
            }
            (Some(id), None)
        }
        Err(error) => (None, Some(error)),
    };
    Ok(Some(OfficeSaved {
        path,
        result_id,
        registration_error,
    }))
}

#[derive(Deserialize, Serialize)]
struct Publication {
    target: PathBuf,
    sha: [u8; 32],
    identity: (u64, u64),
}

impl Publication {
    fn new(target: &Path, bytes: &[u8], metadata: &fs::Metadata) -> Result<Self, String> {
        Ok(Self {
            target: target.to_path_buf(),
            sha: Sha256::digest(bytes).into(),
            identity: identity(metadata)?,
        })
    }

    fn matches(&self) -> Result<bool, String> {
        let metadata = match fs::symlink_metadata(&self.target) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
        };
        if identity(&metadata)? != self.identity {
            return Ok(false);
        }
        let (bytes, current) = work::read_with_metadata(&self.target)?;
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        Ok(identity(&current)? == self.identity && digest == self.sha)
    }
}

fn identity(metadata: &fs::Metadata) -> Result<(u64, u64), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err("当前平台暂不支持 Office 成果恢复".into())
    }
}

fn publication_root() -> Result<PathBuf, String> {
    kodo_core::settings::state_dir()
        .map(|root| root.join("office-publications"))
        .ok_or_else(|| "Office 成果记录目录不可用".to_owned())
}

fn publication_path(id: &str) -> Result<PathBuf, String> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| "Office 成果编号无效")?;
    Ok(publication_root()?.join(format!("{id}.json")))
}

fn write_publication(id: &str, record: &Publication) -> Result<PathBuf, String> {
    let path = publication_path(id)?;
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|error| error.to_string())?;
    let result = serde_json::to_vec(record)
        .map_err(|error| error.to_string())
        .and_then(|bytes| file.write_all(&bytes).map_err(|error| error.to_string()))
        .and_then(|_| file.sync_all().map_err(|error| error.to_string()))
        .and_then(|_| work::sync_directory(parent));
    if let Err(error) = result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

fn read_publication(id: &str) -> Result<Publication, String> {
    let data = fs::read(publication_path(id)?).map_err(|error| error.to_string())?;
    serde_json::from_slice(&data).map_err(|error| error.to_string())
}

fn register_result(app: &AppHandle, path: &str, id: &str) -> Result<String, String> {
    app.state::<AppService>()
        .store()?
        .create_artifact(&ArtifactDraft {
            title: Path::new(path).file_name().map_or_else(
                || path.to_owned(),
                |name| name.to_string_lossy().into_owned(),
            ),
            kind: ArtifactKind::File,
            body: None,
            file_path: Some(path.to_owned()),
            source: Some("office".to_owned()),
            category: Some("Office".to_owned()),
            request_id: Some(id.to_owned()),
        })
        .map(|artifact| artifact.id)
        .map_err(|error| error.code().to_owned())
}

pub fn recover_publications(app: AppHandle) {
    let Ok(root) = publication_root() else {
        return;
    };
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(record) = read_publication(id) else {
            continue;
        };
        if record.matches() == Ok(true)
            && record
                .target
                .parent()
                .is_some_and(|parent| work::sync_directory(parent).is_ok())
            && register_result(&app, &record.target.to_string_lossy(), id).is_ok()
        {
            let _ = fs::remove_file(path);
        }
    }
}

#[tauri::command]
pub fn office_open_external(
    app: AppHandle,
    state: State<'_, OfficeService>,
    handle: String,
) -> Result<(), String> {
    let source = state.source(&handle)?;
    source.bytes()?;
    app.opener()
        .open_path(source.path.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn office_close(state: State<'_, OfficeService>, handle: String) {
    state.sources.lock().remove(&handle);
    state.drafts.lock().remove(&handle);
}

#[tauri::command]
pub async fn office_render(
    app: AppHandle,
    handle: String,
    draft: bool,
    page: usize,
) -> Result<OfficePreview, String> {
    if !(1..=500).contains(&page) {
        return Err("预览页码超出范围".into());
    }
    let state = app.state::<OfficeService>();
    let (format, bytes, cache) = if draft {
        let pending = state.draft(&handle)?;
        (
            pending.content.format,
            pending.content.bytes.clone(),
            pending.preview.clone(),
        )
    } else {
        let source = state.source(&handle)?;
        (
            Format::of(&source.path).ok_or("Office 文件格式无效")?,
            source.bytes()?,
            source.preview,
        )
    };
    tauri::async_runtime::spawn_blocking(move || render(&cache, format, &bytes, page))
        .await
        .map_err(|error| error.to_string())?
}

type PreviewCache = Arc<Mutex<Option<RenderedOffice>>>;

struct RenderedOffice {
    scratch: Scratch,
    sandbox: SandboxContext,
    environment: Vec<(String, String)>,
    pdftoppm: PathBuf,
    pages: usize,
    version: String,
    previewed: bool,
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn renderer(name: &str) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    if name == "soffice" {
        let office = Path::new("/Applications/LibreOffice.app/Contents/MacOS/soffice");
        if office.is_file() {
            return office.canonicalize().map_err(|error| error.to_string());
        }
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
    .into_iter()
    .chain(std::env::split_paths(&path))
    .filter(|directory| directory.is_absolute())
    .map(|directory| directory.join(name))
    .find_map(|path| path.is_file().then(|| path.canonicalize().ok()).flatten())
    .ok_or_else(|| format!("本机缺少 Office 预览工具 {name}"))
}

fn renderer_policy(root: &Path, programs: &[&Path]) -> SandboxPolicy {
    let mut policy = SandboxPolicy::for_workspace(root).with_isolated_reads();
    for system in [
        "/System",
        "/usr/bin",
        "/usr/lib",
        "/usr/share",
        "/bin",
        "/dev",
        "/Library/Fonts",
        "/private/etc/localtime",
        "/private/etc/fonts",
        "/opt/homebrew/Cellar",
        "/opt/homebrew/lib",
        "/opt/homebrew/share/poppler",
        "/opt/homebrew/etc/fonts",
        "/usr/local/lib",
        "/usr/local/share/poppler",
    ] {
        policy = policy.with_read_root(system);
    }
    for program in programs {
        let install_root = program
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"));
        if let Some(bundle) = install_root {
            policy = policy.with_read_root(bundle);
        } else {
            policy = policy.with_read_root(program);
        }
    }
    policy
}

fn render(
    cache: &PreviewCache,
    format: Format,
    bytes: &[u8],
    page: usize,
) -> Result<OfficePreview, String> {
    let mut cached = cache.lock();
    if cached.is_none() {
        *cached = Some(convert(format, bytes)?);
    }
    let document = cached.as_mut().unwrap();
    let preview = document.page(page)?;
    document.previewed = true;
    Ok(preview)
}

fn convert(format: Format, bytes: &[u8]) -> Result<RenderedOffice, String> {
    let file = OfficeFile::from_bytes(format, bytes)?;
    let prepared = file.preview_bytes()?;
    let path = std::env::temp_dir().join(format!("kodo-office-{}", uuid::Uuid::new_v4()));
    let mut directory = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&path).map_err(|error| error.to_string())?;
    let scratch = Scratch(path);
    let root = scratch
        .0
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let source = format!("input.{}", format.extension());
    fs::write(root.join(&source), prepared).map_err(|error| error.to_string())?;
    let profile =
        tauri::Url::from_directory_path(root.join("profile")).map_err(|_| "预览配置路径无效")?;
    let profile = format!("-env:UserInstallation={profile}");
    static RUNTIME: OnceLock<Arc<SandboxRuntime>> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(SandboxRuntime::detect).clone();
    let soffice = renderer("soffice")?;
    let pdfinfo = renderer("pdfinfo")?;
    let pdftoppm = renderer("pdftoppm")?;
    let policy = renderer_policy(&root, &[&soffice, &pdfinfo, &pdftoppm]);
    let sandbox = SandboxContext::new(runtime, policy);
    let mut environment = vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned())];
    #[cfg(target_os = "macos")]
    {
        let fonts = root.join("fonts.conf");
        fs::write(&fonts, r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>/System/Library/Fonts</dir>
  <dir>/Library/Fonts</dir>
  <cachedir prefix="xdg">fontconfig</cachedir>
  <alias><family>Microsoft YaHei</family><prefer><family>Hiragino Sans GB</family><family>Arial Unicode MS</family></prefer></alias>
  <alias><family>Aptos</family><prefer><family>Helvetica Neue</family><family>Arial Unicode MS</family></prefer></alias>
</fontconfig>"#).map_err(|error| error.to_string())?;
        environment.extend([
            (
                "FONTCONFIG_FILE".to_owned(),
                fonts.to_string_lossy().into_owned(),
            ),
            (
                "FONTCONFIG_PATH".to_owned(),
                root.to_string_lossy().into_owned(),
            ),
            (
                "XDG_CACHE_HOME".to_owned(),
                root.to_string_lossy().into_owned(),
            ),
        ]);
    }
    let mut document = RenderedOffice {
        scratch,
        sandbox,
        environment,
        pdftoppm,
        pages: 0,
        version: file.version,
        previewed: false,
    };
    document.run(
        &soffice,
        &[
            &profile,
            "--headless",
            "--nologo",
            "--norestore",
            "--convert-to",
            "pdf",
            "--outdir",
            ".",
            &source,
        ],
    )?;
    let pdf = root.join("input.pdf");
    let pdf = fs::symlink_metadata(&pdf).map_err(|_| "Office 预览引擎未生成 PDF")?;
    if !pdf.is_file() || pdf.len() == 0 || pdf.len() > 64 * 1024 * 1024 {
        return Err("Office 预览 PDF 为空或超出大小上限".into());
    }
    let info = document.run(&pdfinfo, &["input.pdf"])?;
    document.pages = info
        .lines()
        .find_map(|line| line.strip_prefix("Pages:")?.trim().parse().ok())
        .ok_or("无法读取 Office 预览页数")?;
    Ok(document)
}

impl RenderedOffice {
    fn run(&self, program: &Path, args: &[&str]) -> Result<String, String> {
        let quote = |text: &str| format!("'{}'", text.replace('\'', "'\"'\"'"));
        let mut command = quote(&program.to_string_lossy());
        for arg in args {
            command.push(' ');
            command.push_str(&quote(arg));
        }
        let outcome = ProcessRunner::run(
            &ProcessSpec::shell(&self.scratch.0, command)
                .timeout(Duration::from_secs(25))
                .env(EnvPolicy::Scrubbed {
                    vars: self.environment.clone(),
                })
                .stdout_limit(32 * 1024)
                .stderr_limit(16 * 1024)
                .sandbox(self.sandbox.clone()),
            &|| true,
        );
        if !outcome.status.is_success() {
            return Err(format!(
                "Office 预览工具 {} 执行失败（{}）",
                program.file_name().unwrap().to_string_lossy(),
                outcome
                    .failure
                    .as_ref()
                    .map_or(outcome.status.label(), |error| error.code())
            ));
        }
        Ok(outcome.stdout)
    }

    fn page(&self, page: usize) -> Result<OfficePreview, String> {
        if page > self.pages {
            return Err("Office 预览页不存在".into());
        }
        let number = page.to_string();
        self.run(
            &self.pdftoppm,
            &[
                "-f",
                &number,
                "-l",
                &number,
                "-singlefile",
                "-scale-to",
                "1400",
                "-png",
                "input.pdf",
                "page",
            ],
        )?;
        let image = self.scratch.0.join("page.png");
        let png = work::read_bytes(&image).map_err(|_| "Office 预览图未生成")?;
        let _ = fs::remove_file(image);
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") || png.len() > 4 * 1024 * 1024 {
            return Err("Office 预览图格式或大小不受支持".into());
        }
        Ok(OfficePreview {
            image: format!("data:image/png;base64,{}", crate::base64_encode(&png)),
            pages: self.pages,
            version: self.version.clone(),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires the local Seatbelt probe, LibreOffice and Poppler"]
    fn office_render_smoke_uses_read_isolation_and_generates_page_images() {
        let path = std::env::temp_dir().join(format!("kodo-office-probe-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let scratch = Scratch(path);
        let root = scratch.0.join("renderer");
        fs::create_dir(&root).unwrap();
        let adjacent = scratch.0.join("synthetic-adjacent.txt");
        fs::write(&adjacent, b"must not be readable").unwrap();
        let runtime = SandboxRuntime::detect();
        assert!(runtime.is_available(), "{:?}", runtime.capabilities());
        let context = SandboxContext::new(runtime, renderer_policy(&root, &[]));
        let spec = ProcessSpec::shell(&root, format!("/bin/cat '{}'", adjacent.display()))
            .sandbox(context);
        let denied = ProcessRunner::run(&spec, &|| true);
        assert!(!denied.status.is_success());
        assert!(
            denied.stderr.contains("Operation not permitted"),
            "{denied:?}"
        );

        for (format, content, pages) in [
            (Format::Docx, "# 经营月报\n经营收入 650 万元", 1),
            (
                Format::Xlsx,
                r#"{"sheets":[{"name":"汇总","rows":[[{"type":"text","value":"合计"}],[{"type":"number","value":"650"}]]}]}"#,
                1,
            ),
            (
                Format::Pptx,
                r#"{"slides":[{"title":"经营月报","layout":"cover","body":["十一月"]},{"title":"华南收入领先","layout":"comparison","body":["计划 600 万元"],"secondary":["实际 650 万元"]}]}"#,
                2,
            ),
        ] {
            let draft = Draft::create(format, content).unwrap();
            let preview = render(&PreviewCache::default(), format, &draft.bytes, pages)
                .unwrap_or_else(|error| panic!("{format:?}: {error}"));
            assert_eq!(preview.pages, pages);
            assert!(preview.image.starts_with("data:image/png;base64,iVBOR"));
        }
    }

    #[test]
    fn publication_recovery_does_not_claim_an_identical_foreign_file() {
        let path =
            std::env::temp_dir().join(format!("kodo-office-journal-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let scratch = Scratch(path);
        let stage = scratch.0.join("stage");
        let target = scratch.0.join("target.docx");
        let bytes = b"synthetic";
        fs::write(&stage, bytes).unwrap();
        let record = Publication::new(&target, bytes, &fs::metadata(&stage).unwrap()).unwrap();
        fs::write(&target, bytes).unwrap();
        assert!(!record.matches().unwrap());
        fs::remove_file(&target).unwrap();
        fs::hard_link(&stage, &target).unwrap();
        assert!(record.matches().unwrap());
    }
}
