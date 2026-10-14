import {
  BookOpen,
  Check,
  FileText,
  Search,
  Settings2,
  ShieldCheck,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { isDesktop, workspace, type AppManifestDto, type ProjectDto } from "../../api";
import { T } from "../../i18n";
import { DEMO_KNOWLEDGE_PROJECTS } from "../../data/knowledge";
import {
  knowledgeConfirmImport,
  knowledgeCreateBase,
  knowledgeDeleteDocument,
  knowledgeImportText,
  knowledgeInspectDocument,
  knowledgeListBases,
  knowledgeListDocuments,
  knowledgePickImport,
  knowledgeSearch,
  knowledgeSetEnabled,
  type DocumentDetail,
  type ImportPreview,
  type KnowledgeBase,
  type KnowledgeDocument,
  type KnowledgeHit,
} from "../../knowledgeApi";
import "./knowledge.css";

type Props = {
  manifest: AppManifestDto;
  onBack: () => void;
};

type View = "documents" | "search" | "settings";
const PERSONAL = "__personal__";

function pathFor(scopeValue: string): string | null {
  return scopeValue === PERSONAL ? null : scopeValue;
}

function basesForScope(rows: KnowledgeBase[], projectPath: string | null): KnowledgeBase[] {
  return rows.filter((base) => base.projectPath === projectPath);
}

function errorText(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  const copy: Record<string, string> = {
    invalid_input: "输入不符合要求；文件需为 UTF-8 文本、单文件不超过 256 KiB，且标题和正文不能为空。",
    not_found: "资料或知识库已不存在，请刷新后重试。",
    scope_denied: "当前范围无权访问这份资料。",
    storage_unavailable: "知识库暂时不可用，请稍后重试。",
    revision_conflict: "资料已被其他操作更新，请重新加载后再试。",
    sensitive_content: "文件内容触发了安全限制，未导入。",
    unsupported_format: "暂不支持此格式；目前可导入 Markdown、TXT 和笔记。",
    preview_expired: "导入预览已失效，请重新选择文件。",
    source_changed: "文件在预览后发生变化，请重新选择并确认。",
    knowledge_unavailable: "知识库暂时不可用，请稍后重试。",
  };
  return copy[code] ?? "操作失败，请检查知识库状态后重试。";
}

function sourceLabel(sourceKind: KnowledgeDocument["sourceKind"]): string {
  switch (sourceKind) {
    case "note": return "笔记";
    case "file": return "本地文件快照";
    case "result": return "成果文本";
    case "answer": return "回答文本";
  }
}

function statusLabel(status: KnowledgeDocument["status"]): string {
  switch (status) {
    case "ready": return "可检索 · 关键词";
    case "disabled": return "已停用";
    case "quarantined": return "已隔离";
  }
}

function projectName(path: string | null, projects: ProjectDto[]): string {
  if (path === null) return "个人资料";
  return projects.find((project) => project.path === path)?.name ?? path;
}

function knowledgeViewKey(projectPath: string | null, baseId: string): string {
  return `${projectPath ?? PERSONAL}\u0000${baseId}`;
}

export function KnowledgeApp({ manifest, onBack }: Props) {
  const [projects, setProjects] = useState<ProjectDto[]>([]);
  const [scopeValue, setScopeValue] = useState(PERSONAL);
  const projectPath = pathFor(scopeValue);
  const preferredBaseId = useRef<string | null>(null);
  const [bases, setBases] = useState<KnowledgeBase[]>([]);
  const [baseId, setBaseId] = useState("");
  const [documents, setDocuments] = useState<KnowledgeDocument[]>([]);
  const [loadingBases, setLoadingBases] = useState(true);
  const [loadingDocuments, setLoadingDocuments] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<View>("documents");
  const [filterText, setFilterText] = useState("");
  const [statusFilter, setStatusFilter] = useState("all");
  const [detail, setDetail] = useState<DocumentDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [busyDocumentId, setBusyDocumentId] = useState<string | null>(null);
  const [createBaseOpen, setCreateBaseOpen] = useState(false);
  const [createScopeValue, setCreateScopeValue] = useState(PERSONAL);
  const [baseName, setBaseName] = useState("");
  const [baseDescription, setBaseDescription] = useState("");
  const [noteOpen, setNoteOpen] = useState(false);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteBody, setNoteBody] = useState("");
  const [noteVerified, setNoteVerified] = useState(false);
  const [previews, setPreviews] = useState<ImportPreview[]>([]);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [busyToken, setBusyToken] = useState<string | null>(null);
  const [searchText, setSearchText] = useState("");
  const [searchHits, setSearchHits] = useState<KnowledgeHit[] | null>(null);
  const [searchBusy, setSearchBusy] = useState(false);
  const scopePathRef = useRef(projectPath);
  scopePathRef.current = projectPath;
  const currentViewKey = knowledgeViewKey(projectPath, baseId);
  const currentViewKeyRef = useRef(currentViewKey);
  currentViewKeyRef.current = currentViewKey;
  const viewEpochRef = useRef(0);
  const baseListRequestRef = useRef(0);
  const documentsRequestRef = useRef(0);
  const detailRequestRef = useRef(0);
  const searchRequestRef = useRef(0);
  const pickerRequestRef = useRef(0);
  const confirmRequestRef = useRef(new Map<string, number>());
  const previewScopesRef = useRef(new Map<string, string>());

  const isCurrentView = (key: string, epoch: number) =>
    currentViewKeyRef.current === key && viewEpochRef.current === epoch;

  const clearScopedView = () => {
    setDocuments([]);
    setLoadingDocuments(false);
    setDetail(null);
    setDetailLoading(false);
    setSearchHits(null);
    setSearchBusy(false);
    setPreviews([]);
    setPreviewBusy(false);
    setBusyToken(null);
    setConfirmDeleteId(null);
    setBusyDocumentId(null);
    setNoteTitle("");
    setNoteBody("");
    setNoteVerified(false);
    previewScopesRef.current.clear();
    confirmRequestRef.current.clear();
  };

  const changeScope = (nextScopeValue: string) => {
    if (nextScopeValue === scopeValue) return;
    const nextPath = pathFor(nextScopeValue);
    scopePathRef.current = nextPath;
    currentViewKeyRef.current = knowledgeViewKey(nextPath, "");
    viewEpochRef.current += 1;
    baseListRequestRef.current += 1;
    documentsRequestRef.current += 1;
    detailRequestRef.current += 1;
    searchRequestRef.current += 1;
    pickerRequestRef.current += 1;
    clearScopedView();
    setBases([]);
    setBaseId("");
    setLoadingBases(true);
    setCreateBaseOpen(false);
    setNoteOpen(false);
    setError(null);
    setScopeValue(nextScopeValue);
  };

  const changeBase = (nextBaseId: string) => {
    if (nextBaseId === baseId) return;
    currentViewKeyRef.current = knowledgeViewKey(projectPath, nextBaseId);
    viewEpochRef.current += 1;
    documentsRequestRef.current += 1;
    detailRequestRef.current += 1;
    searchRequestRef.current += 1;
    pickerRequestRef.current += 1;
    clearScopedView();
    setError(null);
    setBaseId(nextBaseId);
  };

  useEffect(() => {
    let live = true;
    void workspace().then(
      (value) => live && setProjects(value?.projects ?? (isDesktop() ? [] : DEMO_KNOWLEDGE_PROJECTS)),
      (failure: unknown) => live && setError(errorText(failure)),
    );
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    let live = true;
    const requestPath = projectPath;
    const requestId = ++baseListRequestRef.current;
    setLoadingBases(true);
    setBases([]);
    setBaseId("");
    setDocuments([]);
    setDetail(null);
    setSearchHits(null);
    setConfirmDeleteId(null);
    void knowledgeListBases({ projectPath }).then(
      (rows) => {
        if (!live || baseListRequestRef.current !== requestId || scopePathRef.current !== requestPath) return;
        const scopedRows = basesForScope(rows, projectPath);
        setBases(scopedRows);
        const preferred = preferredBaseId.current;
        const nextBaseId = preferred && scopedRows.some((row) => row.id === preferred)
          ? preferred
          : scopedRows[0]?.id ?? "";
        currentViewKeyRef.current = knowledgeViewKey(projectPath, nextBaseId);
        viewEpochRef.current += 1;
        setBaseId(nextBaseId);
        preferredBaseId.current = null;
        setLoadingBases(false);
        setError(null);
      },
      (failure: unknown) => {
        if (!live || baseListRequestRef.current !== requestId || scopePathRef.current !== requestPath) return;
        setLoadingBases(false);
        setError(errorText(failure));
      },
    );
    return () => {
      live = false;
    };
  }, [projectPath]);

  useEffect(() => {
    if (!baseId) {
      setDocuments([]);
      setLoadingDocuments(false);
      return;
    }
    let live = true;
    const requestKey = knowledgeViewKey(projectPath, baseId);
    const requestEpoch = viewEpochRef.current;
    const requestId = ++documentsRequestRef.current;
    setLoadingDocuments(true);
    setDetail(null);
    void knowledgeListDocuments({ baseId, projectPath }).then(
      (rows) => {
        if (!live || documentsRequestRef.current !== requestId || !isCurrentView(requestKey, requestEpoch)) return;
        setDocuments(rows);
        setLoadingDocuments(false);
        setError(null);
      },
      (failure: unknown) => {
        if (!live || documentsRequestRef.current !== requestId || !isCurrentView(requestKey, requestEpoch)) return;
        setLoadingDocuments(false);
        setError(errorText(failure));
      },
    );
    return () => {
      live = false;
    };
  }, [baseId, projectPath, refreshKey]);

  useEffect(() => {
    if (!createBaseOpen && !noteOpen && previews.length === 0) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setCreateBaseOpen(false);
      setNoteOpen(false);
      setPreviews([]);
      previewScopesRef.current.clear();
      setError(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [createBaseOpen, noteOpen, previews.length]);

  const refreshDocuments = useCallback(() => setRefreshKey((current) => current + 1), []);

  const createBase = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = baseName.trim();
    const targetScopeValue = createScopeValue;
    const newPath = pathFor(targetScopeValue);
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    if (!name) {
      setError("请输入知识库名称。");
      return;
    }
    setError(null);
    setBusyDocumentId("__create_base__");
    try {
      const base = await knowledgeCreateBase({
        draft: { name, description: baseDescription.trim(), projectPath: newPath },
      });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      preferredBaseId.current = base.id;
      setBusyDocumentId(null);
      setCreateBaseOpen(false);
      setBaseName("");
      setBaseDescription("");
      setScopeValue(targetScopeValue);
      if (newPath === projectPath) {
        const rows = await knowledgeListBases({ projectPath: newPath });
        if (!isCurrentView(requestKey, requestEpoch)) return;
        clearScopedView();
        setBases(basesForScope(rows, newPath));
        currentViewKeyRef.current = knowledgeViewKey(newPath, base.id);
        viewEpochRef.current += 1;
        setBaseId(base.id);
      }
      setError(null);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch)) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch)) setBusyDocumentId(null);
    }
  };

  const saveNote = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!baseId) {
      setError("请先创建或选择知识库。");
      return;
    }
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    setError(null);
    setBusyDocumentId("__note__");
    try {
      await knowledgeImportText({
        baseId,
        projectPath,
        draft: {
          title: noteTitle.trim(),
          body: noteBody,
          sourceKind: "note",
          sourceRef: null,
          verified: noteVerified,
        },
      });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      setNoteOpen(false);
      setNoteTitle("");
      setNoteBody("");
      setNoteVerified(false);
      refreshDocuments();
      setError(null);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch)) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch)) setBusyDocumentId(null);
    }
  };

  const chooseFiles = async () => {
    if (!baseId) {
      setError("请先创建或选择知识库。");
      return;
    }
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    const requestId = ++pickerRequestRef.current;
    setError(null);
    setPreviews([]);
    previewScopesRef.current.clear();
    setPreviewBusy(true);
    try {
      const rows = await knowledgePickImport({ baseId, projectPath });
      if (!isCurrentView(requestKey, requestEpoch) || pickerRequestRef.current !== requestId) return;
      for (const row of rows) previewScopesRef.current.set(row.token, requestKey);
      setPreviews(rows);
      if (rows.length === 0) setError(null);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch) && pickerRequestRef.current === requestId) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch) && pickerRequestRef.current === requestId) setPreviewBusy(false);
    }
  };

  const confirmFile = async (preview: ImportPreview) => {
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    if (previewScopesRef.current.get(preview.token) !== requestKey) return;
    const requestId = (confirmRequestRef.current.get(preview.token) ?? 0) + 1;
    confirmRequestRef.current.set(preview.token, requestId);
    setError(null);
    setBusyToken(preview.token);
    try {
      await knowledgeConfirmImport({ token: preview.token });
      if (!isCurrentView(requestKey, requestEpoch) || confirmRequestRef.current.get(preview.token) !== requestId) return;
      setBusyToken(null);
      previewScopesRef.current.delete(preview.token);
      confirmRequestRef.current.delete(preview.token);
      setPreviews((current) => current.filter((row) => row.token !== preview.token));
      refreshDocuments();
      setError(null);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch) && confirmRequestRef.current.get(preview.token) === requestId) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch) && confirmRequestRef.current.get(preview.token) === requestId) setBusyToken(null);
    }
  };

  const openDetail = async (document: KnowledgeDocument) => {
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    const requestId = ++detailRequestRef.current;
    setError(null);
    setDetail(null);
    setDetailLoading(true);
    try {
      const full = await knowledgeInspectDocument({ id: document.id, projectPath });
      if (!isCurrentView(requestKey, requestEpoch) || detailRequestRef.current !== requestId) return;
      setDetail(full);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch) && detailRequestRef.current === requestId) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch) && detailRequestRef.current === requestId) setDetailLoading(false);
    }
  };

  const toggleDocument = async (document: KnowledgeDocument) => {
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    setError(null);
    setBusyDocumentId(document.id);
    try {
      const next = await knowledgeSetEnabled({
        id: document.id,
        projectPath,
        expectedVersion: document.version,
        enabled: document.status === "disabled",
      });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      setDocuments((current) => current.map((row) => row.id === next.id ? next : row));
      setDetail((current) => current?.document.id === next.id ? { ...current, document: next } : current);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch)) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch)) setBusyDocumentId(null);
    }
  };

  const removeDocument = async (document: KnowledgeDocument) => {
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    setError(null);
    setBusyDocumentId(document.id);
    try {
      await knowledgeDeleteDocument({
        id: document.id,
        projectPath,
        expectedVersion: document.version,
      });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      setDocuments((current) => current.filter((row) => row.id !== document.id));
      setDetail((current) => current?.document.id === document.id ? null : current);
      setConfirmDeleteId(null);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch)) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch)) setBusyDocumentId(null);
    }
  };

  const runSearch = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    const requestId = ++searchRequestRef.current;
    if (!baseId) {
      setError("请先创建或选择知识库。");
      return;
    }
    if (!searchText.trim()) {
      setSearchHits([]);
      setSearchBusy(false);
      setError(null);
      return;
    }
    setSearchBusy(true);
    setError(null);
    try {
      const rows = await knowledgeSearch({
        query: {
          baseIds: [baseId],
          projectPath,
          text: searchText.trim(),
          limit: 8,
          budgetTokens: 1800,
        },
      });
      if (!isCurrentView(requestKey, requestEpoch) || searchRequestRef.current !== requestId) return;
      setSearchHits(rows);
    } catch (failure) {
      if (!isCurrentView(requestKey, requestEpoch) || searchRequestRef.current !== requestId) return;
      setSearchHits(null);
      setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch) && searchRequestRef.current === requestId) setSearchBusy(false);
    }
  };

  const visibleDocuments = documents.filter((document) => {
    const textMatches = !filterText.trim() || document.title.toLocaleLowerCase().includes(filterText.trim().toLocaleLowerCase());
    const statusMatches = statusFilter === "all" || document.status === statusFilter;
    return textMatches && statusMatches;
  });
  const selectedBase = bases.find((base) => base.id === baseId);

  return (
    <main className="main page-app page-knowledge" data-testid="knowledge-app">
      <header className="page-head">
        <div className="page-head-titles">
          <p className="knowledge-breadcrumb">工作台 / 资料</p>
          <h1>{manifest.name}</h1>
          <p className="page-subtitle">让资料成为对话和任务中可追溯的依据。</p>
        </div>
        <div className="page-head-actions">
          <button type="button" className="btn" data-testid="app-back" onClick={onBack}>{T.workbench.back}</button>
          <button
            type="button"
            className="btn btn--primary"
            data-testid="knowledge-new-base"
            onClick={() => {
              setCreateScopeValue(scopeValue);
              setCreateBaseOpen(true);
              setError(null);
            }}
          >
            <BookOpen size={14} strokeWidth={1.8} />新建知识库
          </button>
        </div>
      </header>

      <div className="app-body knowledge-body">
        {error && <p className="page-error" role="alert" data-testid="knowledge-error">{error}</p>}

        <div className="knowledge-contextbar">
          <label className="knowledge-scope-field">
            <span>当前范围</span>
            <select
              className="select"
              aria-label="知识库范围"
              data-testid="knowledge-scope"
              value={scopeValue}
              onChange={(event) => changeScope(event.target.value)}
            >
              <option value={PERSONAL}>个人资料</option>
              {projects.map((project) => <option key={project.path} value={project.path}>{project.name}</option>)}
            </select>
          </label>
          <span className="knowledge-scope-path" title={projectPath ?? "个人范围"}>
            {projectPath ?? "个人范围"}
          </span>
          <span className="spacer" />
          <span className="knowledge-index-label"><BookOpen size={14} />本地索引 <strong>关键词</strong></span>
        </div>

        <nav className="knowledge-tabs" role="tablist" aria-label="知识库页面">
          <button type="button" role="tab" aria-selected={view === "documents"} className={`app-filter${view === "documents" ? " app-filter--active" : ""}`} data-testid="knowledge-tab-documents" onClick={() => { setView("documents"); setError(null); }}>资料</button>
          <button type="button" role="tab" aria-selected={view === "search"} className={`app-filter${view === "search" ? " app-filter--active" : ""}`} data-testid="knowledge-tab-search" onClick={() => { setView("search"); setError(null); }}>检索验证</button>
          <button type="button" role="tab" aria-selected={view === "settings"} className={`app-filter${view === "settings" ? " app-filter--active" : ""}`} data-testid="knowledge-tab-settings" onClick={() => { setView("settings"); setError(null); }}><Settings2 size={13} />设置</button>
        </nav>

        {view === "documents" && (
          <section className="knowledge-view" role="tabpanel" aria-label="资料">
            <div className="knowledge-librarybar">
              <label className="knowledge-base-field">
                <span>当前知识库</span>
                <select
                  className="select"
                  aria-label="当前知识库"
                  data-testid="knowledge-base"
                  value={baseId}
                  disabled={bases.length === 0}
                  onChange={(event) => changeBase(event.target.value)}
                >
                  {bases.length === 0 && <option value="">{loadingBases ? "正在读取…" : "暂无知识库"}</option>}
                  {bases.map((base) => <option key={base.id} value={base.id}>{base.name}</option>)}
                </select>
              </label>
              <span className="knowledge-description">{selectedBase?.description || "在此范围管理已确认导入的资料"}</span>
              <span className="spacer" />
              <button type="button" className="btn" data-testid="knowledge-import-note" disabled={!baseId} onClick={() => { setNoteOpen(true); setError(null); }}>
                <FileText size={14} />新建笔记
              </button>
              <button type="button" className="btn btn--primary" data-testid="knowledge-import-file" disabled={!baseId || previewBusy} onClick={() => void chooseFiles()}>
                <Upload size={14} />{previewBusy ? "读取中…" : "导入资料"}
              </button>
            </div>
            <p className="app-hint knowledge-import-hint">仅支持 Markdown、TXT 纯 UTF-8 文本；单个文件不超过 256 KiB。</p>

            {bases.length === 0 && !loadingBases ? (
              <div className="knowledge-empty" data-testid="knowledge-no-bases">
                <BookOpen size={21} strokeWidth={1.6} />
                <strong>这个范围还没有知识库</strong>
                <p>创建知识库后，可以加入 Markdown、TXT、笔记或成果中心文本。</p>
                <button type="button" className="btn btn--primary" onClick={() => { setCreateScopeValue(scopeValue); setCreateBaseOpen(true); }}>新建知识库</button>
              </div>
            ) : (
              <div className={`knowledge-columns${detail ? " knowledge-columns--detail" : ""}`}>
                <section className="knowledge-list-panel" aria-label="资料列表">
                  <div className="knowledge-list-toolbar">
                    <label className="knowledge-search-field">
                      <Search size={14} aria-hidden="true" />
                      <input
                        type="search"
                        value={filterText}
                        aria-label="筛选资料名称"
                        placeholder="筛选资料名称…"
                        data-testid="knowledge-document-filter"
                        onChange={(event) => { setFilterText(event.target.value); setError(null); }}
                      />
                    </label>
                    <label className="knowledge-status-field">
                      <span className="sr-only">资料状态</span>
                      <select className="select" aria-label="资料状态" data-testid="knowledge-status-filter" value={statusFilter} onChange={(event) => setStatusFilter(event.target.value)}>
                        <option value="all">全部状态</option>
                        <option value="ready">可检索</option>
                        <option value="disabled">已停用</option>
                        <option value="quarantined">已隔离</option>
                      </select>
                    </label>
                  </div>
                  <div className="knowledge-list-count">
                    <span>{loadingDocuments ? "正在读取资料…" : `${visibleDocuments.length} 份资料`}</span>
                    <span>点击资料查看原文快照</span>
                  </div>
                  {visibleDocuments.length === 0 && !loadingDocuments ? (
                    <div className="knowledge-empty knowledge-empty--compact" data-testid="knowledge-empty-documents">
                      <p>{documents.length === 0 ? "这个知识库还没有资料。" : "没有符合条件的资料。"}</p>
                      {documents.length === 0 && <button type="button" className="btn" onClick={() => { setNoteOpen(true); setError(null); }}>新建第一条笔记</button>}
                    </div>
                  ) : (
                    <ul className="knowledge-list">
                      {visibleDocuments.map((document) => (
                        <li key={document.id} className={`knowledge-row${detail?.document.id === document.id ? " knowledge-row--selected" : ""}`} data-testid={`knowledge-document-${document.id}`}>
                          <button type="button" className="knowledge-row-open" data-testid={`knowledge-open-${document.id}`} onClick={() => void openDetail(document)}>
                            <span className="knowledge-file-badge">{document.title.toLocaleLowerCase().endsWith(".md") ? "MD" : document.title.toLocaleLowerCase().endsWith(".txt") ? "TXT" : "文"}</span>
                            <span className="knowledge-row-copy">
                              <strong title={document.title}>{document.title}</strong>
                              <span className="knowledge-row-meta">
                                <span className={`knowledge-status knowledge-status--${document.status}`}>{statusLabel(document.status)}</span>
                                <span>{sourceLabel(document.sourceKind)}</span>
                                <span>{document.verified ? "已核实" : "待核实"}</span>
                                <span>v{document.version}</span>
                              </span>
                            </span>
                          </button>
                          <div className="knowledge-row-actions">
                            <button
                              type="button"
                              className="btn knowledge-row-action"
                              disabled={busyDocumentId === document.id || document.status === "quarantined"}
                              data-testid={`knowledge-toggle-${document.id}`}
                              onClick={() => void toggleDocument(document)}
                            >
                              {busyDocumentId === document.id ? "保存中…" : document.status === "disabled" ? "启用" : "停用"}
                            </button>
                            {confirmDeleteId === document.id ? (
                              <button type="button" className="btn btn--danger knowledge-row-action" disabled={busyDocumentId === document.id} data-testid={`knowledge-delete-confirm-${document.id}`} onClick={() => void removeDocument(document)}>
                                确认删除
                              </button>
                            ) : (
                              <button type="button" className="icon-btn icon-btn--sm" aria-label={`删除 ${document.title}`} title="删除资料" data-testid={`knowledge-delete-${document.id}`} onClick={() => { setConfirmDeleteId(document.id); setError(null); }}>
                                <Trash2 size={13} strokeWidth={1.8} />
                              </button>
                            )}
                          </div>
                          {confirmDeleteId === document.id && <p className="knowledge-delete-hint" role="alert">删除会移除知识库快照，原始文件不会被修改。</p>}
                        </li>
                      ))}
                    </ul>
                  )}
                </section>

                {detail && (
                  <aside className="knowledge-detail" aria-label="资料详情" data-testid="knowledge-detail">
                    <header className="knowledge-detail-head">
                      <strong>资料详情</strong>
                      <button type="button" className="icon-btn icon-btn--sm" aria-label="关闭资料详情" onClick={() => setDetail(null)}><X size={14} /></button>
                    </header>
                    {detailLoading ? <p className="app-hint">正在读取原文快照…</p> : (
                      <>
                        <h2>{detail.document.title}</h2>
                        <div className="knowledge-detail-badges">
                          <span className={`knowledge-status knowledge-status--${detail.document.status}`}>{statusLabel(detail.document.status)}</span>
                          <span>{detail.document.verified ? "已核实" : "待核实"}</span>
                        </div>
                        <dl className="knowledge-detail-meta">
                          <div><dt>来源</dt><dd>{sourceLabel(detail.document.sourceKind)}</dd></div>
                          <div><dt>范围</dt><dd>{projectName(projectPath, projects)}</dd></div>
                          <div><dt>版本</dt><dd>v{detail.document.version}</dd></div>
                          <div><dt>内容</dt><dd>{detail.document.chunkCount} 个片段</dd></div>
                        </dl>
                        <div className="knowledge-detail-divider" />
                        <h3>原文快照</h3>
                        <div className="knowledge-source-lines" data-testid="knowledge-source-lines">
                          {detail.body.split("\n").map((line, index) => (
                            <div className="knowledge-source-line" key={`${index}-${line}`}>
                              <span aria-hidden="true">{index + 1}</span><code>{line || " "}</code>
                            </div>
                          ))}
                        </div>
                        <h3>分块定位</h3>
                        <ol className="knowledge-chunks">
                          {detail.chunks.map((chunk) => (
                            <li key={chunk.id}>
                              <strong>{chunk.heading}</strong>
                              <span>第 {chunk.lineStart}–{chunk.lineEnd} 行</span>
                            </li>
                          ))}
                        </ol>
                        <p className="app-hint">正文来自已保存的不可变快照。停用后仍可查看本次快照，但不会参与新检索。</p>
                      </>
                    )}
                  </aside>
                )}
              </div>
            )}
          </section>
        )}

        {view === "search" && (
          <section className="knowledge-search-view" role="tabpanel" aria-label="检索验证">
            <div className="knowledge-view-intro">
              <h2>检索验证</h2>
              <p>只搜索当前选中的知识库，使用关键词检索；不会自动绑定到任何会话。</p>
            </div>
            <form className="knowledge-search-form" onSubmit={(event) => void runSearch(event)}>
              <label className="knowledge-search-field knowledge-search-field--large">
                <Search size={15} aria-hidden="true" />
                <input
                  type="search"
                  autoFocus
                  value={searchText}
                  aria-label="检索知识库"
                  placeholder="输入关键词，例如：旧客户端"
                  data-testid="knowledge-search-input"
                  onChange={(event) => { searchRequestRef.current += 1; setSearchBusy(false); setSearchText(event.target.value); setSearchHits(null); setError(null); }}
                />
              </label>
              <button type="submit" className="btn btn--primary" disabled={searchBusy || !baseId} data-testid="knowledge-search-submit">
                {searchBusy ? "检索中…" : "检索"}
              </button>
            </form>
            {searchHits === null ? (
              <div className="knowledge-empty knowledge-search-empty"><Search size={20} /><p>检索结果会显示原文片段、标题和行号。</p></div>
            ) : searchHits.length === 0 ? (
              <p className="page-empty" data-testid="knowledge-search-empty">没有找到匹配资料。</p>
            ) : (
              <ol className="knowledge-hit-list" data-testid="knowledge-search-results">
                {searchHits.map((hit) => (
                  <li key={hit.chunkId} className="knowledge-hit">
                    <div className="knowledge-hit-head">
                      <FileText size={14} />
                      <strong>{hit.title}</strong>
                      <span>{hit.heading} · 第 {hit.lineStart}–{hit.lineEnd} 行</span>
                      <span className="knowledge-keyword-tag">关键词</span>
                    </div>
                    <blockquote>{hit.quote}</blockquote>
                  </li>
                ))}
              </ol>
            )}
          </section>
        )}

        {view === "settings" && (
          <section className="knowledge-settings" role="tabpanel" aria-label="知识库设置" data-testid="knowledge-settings">
            <div className="knowledge-view-intro">
              <h2>范围与索引</h2>
              <p>资料按所选范围隔离，检索只使用明确选中的知识库。</p>
            </div>
            <div className="knowledge-setting-row">
              <span className="knowledge-setting-icon"><ShieldCheck size={16} /></span>
              <div><h3>当前资料范围</h3><p>{projectName(projectPath, projects)}{projectPath ? ` · ${projectPath}` : " · 个人范围"}</p></div>
            </div>
            <div className="knowledge-setting-row">
              <span className="knowledge-setting-icon"><Search size={16} /></span>
              <div><h3>索引方式</h3><p>关键词检索。当前版本不使用向量、语义重排或远程索引。</p></div>
            </div>
            <div className="knowledge-setting-row">
              <span className="knowledge-setting-icon"><FileText size={16} /></span>
              <div><h3>支持的来源</h3><p>Markdown、TXT、手动笔记和成果中心文本。PDF、DOCX 与 OCR 暂不支持。</p></div>
            </div>
            <div className="knowledge-setting-row">
              <span className="knowledge-setting-icon"><Check size={16} /></span>
              <div><h3>会话使用</h3><p>新会话默认不绑定知识库。只有在会话中明确选择后，资料才会进入本轮检索。</p></div>
            </div>
            <p className="knowledge-scope-disclosure">个人知识库可以在其他项目中使用，但必须在会话或检索操作中显式选择；项目知识库仅在其注册项目范围内可用。停用或删除资料后，新检索会排除该资料。</p>
          </section>
        )}
      </div>

      {createBaseOpen && (
        <div className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="knowledge-create-title" data-testid="knowledge-create-dialog">
          <div className="settings-modal-backdrop" />
          <form className="settings-modal-panel wb-modal-panel knowledge-dialog" onSubmit={(event) => void createBase(event)}>
            <header className="modal-head">
              <h2 className="modal-title" id="knowledge-create-title">新建知识库</h2>
              <span className="spacer" />
              <button type="button" className="icon-btn icon-btn--sm" aria-label="关闭新建知识库" onClick={() => { setCreateBaseOpen(false); setError(null); }}><X size={14} /></button>
            </header>
            <div className="skill-detail-body knowledge-dialog-body">
              {error && <p className="page-error" role="alert">{error}</p>}
              <label className="field field--wide">
                <span className="field-label">名称</span>
                <input className="input" autoFocus required value={baseName} aria-label="知识库名称" data-testid="knowledge-create-name" onChange={(event) => { setBaseName(event.target.value); setError(null); }} />
              </label>
              <label className="field field--wide">
                <span className="field-label">说明</span>
                <textarea className="knowledge-textarea" value={baseDescription} aria-label="知识库说明" data-testid="knowledge-create-description" onChange={(event) => { setBaseDescription(event.target.value); setError(null); }} />
              </label>
              <label className="field field--wide">
                <span className="field-label">范围</span>
                <select className="select" value={createScopeValue} aria-label="新知识库范围" data-testid="knowledge-create-scope" onChange={(event) => { setCreateScopeValue(event.target.value); setError(null); }}>
                  <option value={PERSONAL}>个人资料</option>
                  {projects.map((project) => <option key={project.path} value={project.path}>{project.name}</option>)}
                </select>
              </label>
              <p className="app-hint">项目选项只来自工作台已注册的项目。</p>
            </div>
            <footer className="skill-detail-foot">
              <span className="spacer" />
              <button type="button" className="btn" onClick={() => { setCreateBaseOpen(false); setError(null); }}>取消</button>
              <button type="submit" className="btn btn--primary" disabled={!baseName.trim() || busyDocumentId === "__create_base__"} data-testid="knowledge-create-submit">
                {busyDocumentId === "__create_base__" ? "创建中…" : "创建知识库"}
              </button>
            </footer>
          </form>
        </div>
      )}

      {noteOpen && (
        <div className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="knowledge-note-title" data-testid="knowledge-note-dialog">
          <div className="settings-modal-backdrop" />
          <form className="settings-modal-panel wb-modal-panel knowledge-dialog" onSubmit={(event) => void saveNote(event)}>
            <header className="modal-head">
              <h2 className="modal-title" id="knowledge-note-title">新建笔记</h2>
              <span className="spacer" />
              <button type="button" className="icon-btn icon-btn--sm" aria-label="关闭新建笔记" onClick={() => { setNoteOpen(false); setError(null); }}><X size={14} /></button>
            </header>
            <div className="skill-detail-body knowledge-dialog-body">
              {error && <p className="page-error" role="alert">{error}</p>}
              <label className="field field--wide">
                <span className="field-label">标题</span>
                <input className="input" autoFocus required value={noteTitle} aria-label="笔记标题" data-testid="knowledge-note-title-input" onChange={(event) => { setNoteTitle(event.target.value); setError(null); }} />
              </label>
              <label className="field field--wide">
                <span className="field-label">正文</span>
                <textarea className="knowledge-textarea knowledge-textarea--body" required value={noteBody} aria-label="笔记正文" data-testid="knowledge-note-body" onChange={(event) => { setNoteBody(event.target.value); setError(null); }} />
              </label>
              <label className="knowledge-check">
                <input type="checkbox" checked={noteVerified} onChange={(event) => setNoteVerified(event.target.checked)} data-testid="knowledge-note-verified" />
                <span>我已核实这份内容</span>
              </label>
            </div>
            <footer className="skill-detail-foot">
              <span className="spacer" />
              <button type="button" className="btn" onClick={() => { setNoteOpen(false); setError(null); }}>取消</button>
              <button type="submit" className="btn btn--primary" disabled={!noteTitle.trim() || !noteBody.trim() || busyDocumentId === "__note__"} data-testid="knowledge-note-submit">
                {busyDocumentId === "__note__" ? "保存中…" : "保存笔记"}
              </button>
            </footer>
          </form>
        </div>
      )}

      {previews.length > 0 && (
        <div className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="knowledge-import-preview-title" data-testid="knowledge-import-preview">
          <div className="settings-modal-backdrop" />
          <section className="settings-modal-panel wb-modal-panel knowledge-dialog">
            <header className="modal-head">
              <h2 className="modal-title" id="knowledge-import-preview-title">导入预览</h2>
              <span className="spacer" />
              <button type="button" className="icon-btn icon-btn--sm" aria-label="关闭导入预览" onClick={() => { setPreviews([]); setError(null); }}><X size={14} /></button>
            </header>
            <div className="skill-detail-body knowledge-dialog-body">
              {error && <p className="page-error" role="alert">{error}</p>}
              <p className="app-hint">确认后保存预览对应的文件快照。支持 Markdown 与 TXT；PDF、DOCX、图片和 OCR 暂不支持。</p>
              {previews.map((preview) => (
                <article className="knowledge-preview-card" key={preview.token} data-testid={`knowledge-preview-${preview.token}`}>
                  <div className="knowledge-preview-card-head">
                    <strong>{preview.title}</strong>
                    <span>{preview.characterCount.toLocaleString()} 字符 · {preview.chunkCount} 个片段</span>
                  </div>
                  <pre>{preview.bodyPreview}</pre>
                  <button type="button" className="btn btn--primary" disabled={busyToken === preview.token} data-testid={`knowledge-confirm-import-${preview.token}`} onClick={() => void confirmFile(preview)}>
                    {busyToken === preview.token ? "保存中…" : "确认导入此快照"}
                  </button>
                </article>
              ))}
            </div>
            <footer className="skill-detail-foot">
              <span className="app-hint">确认令牌绑定了所选知识库和预览内容。</span>
              <span className="spacer" />
              <button type="button" className="btn" onClick={() => { setPreviews([]); setError(null); }}>完成</button>
            </footer>
          </section>
        </div>
      )}
    </main>
  );
}
