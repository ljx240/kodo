import { useEffect, useRef, useState } from "react";
import { isDesktop, workspace } from "../../api";
import { DEMO_KNOWLEDGE_PROJECTS } from "../../data/knowledge";
import {
  knowledgeCreateBase,
  knowledgeImportText,
  knowledgeListBases,
  type KnowledgeBase,
  type KnowledgeSourceKind,
} from "../../knowledgeApi";

type Props = {
  title: string;
  body: string;
  sourceKind: Extract<KnowledgeSourceKind, "result" | "answer">;
  sourceRef: string;
  onClose: () => void;
  onSaved?: () => void;
};

const PERSONAL = "__personal__";

function basesForScope(rows: KnowledgeBase[], projectPath: string | null): KnowledgeBase[] {
  return rows.filter((base) => base.projectPath === projectPath);
}

function saveViewKey(projectPath: string | null, baseId: string): string {
  return `${projectPath ?? PERSONAL}\u0000${baseId}`;
}

function errorText(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  const copy: Record<string, string> = {
    invalid_input: "标题、正文不能为空；正文需为 UTF-8 文本且不超过 256 KiB。",
    not_found: "所选知识库已不存在，请刷新后重试。",
    scope_denied: "当前范围无法访问这个知识库。",
    storage_unavailable: "知识库暂时不可用，请稍后重试。",
    sensitive_content: "内容包含暂不允许导入的信息。",
    unsupported_format: "当前只支持 Markdown、TXT、笔记和文本成果。",
  };
  return copy[code] ?? "保存失败，请检查知识库状态后重试。";
}

export function KnowledgeSaveDialog({
  title,
  body,
  sourceKind,
  sourceRef,
  onClose,
  onSaved,
}: Props) {
  const [projects, setProjects] = useState<Array<{ path: string; name: string }>>([]);
  const [scopeValue, setScopeValue] = useState(PERSONAL);
  const [bases, setBases] = useState<KnowledgeBase[]>([]);
  const [baseId, setBaseId] = useState("");
  const [verified, setVerified] = useState(false);
  const [createBase, setCreateBase] = useState(false);
  const [newBaseName, setNewBaseName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const projectPath = scopeValue === PERSONAL ? null : scopeValue;
  const scopePathRef = useRef(projectPath);
  scopePathRef.current = projectPath;
  const currentViewKey = saveViewKey(projectPath, baseId);
  const currentViewKeyRef = useRef(currentViewKey);
  currentViewKeyRef.current = currentViewKey;
  const viewEpochRef = useRef(0);
  const baseListRequestRef = useRef(0);
  const mountedRef = useRef(false);
  const isCurrentView = (key: string, epoch: number) =>
    mountedRef.current && currentViewKeyRef.current === key && viewEpochRef.current === epoch;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      viewEpochRef.current += 1;
      baseListRequestRef.current += 1;
    };
  }, []);

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
    if (busy) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose]);

  useEffect(() => {
    let live = true;
    const requestPath = projectPath;
    const requestId = ++baseListRequestRef.current;
    void knowledgeListBases({ projectPath }).then(
      (rows) => {
        if (!live || baseListRequestRef.current !== requestId || scopePathRef.current !== requestPath) return;
        const scopedRows = basesForScope(rows, projectPath);
        setBases(scopedRows);
        const nextBaseId = scopedRows[0]?.id ?? "";
        currentViewKeyRef.current = saveViewKey(projectPath, nextBaseId);
        viewEpochRef.current += 1;
        setBaseId(nextBaseId);
        setError(null);
      },
      (failure: unknown) => {
        if (live && baseListRequestRef.current === requestId && scopePathRef.current === requestPath) setError(errorText(failure));
      },
    );
    return () => {
      live = false;
    };
  }, [projectPath]);

  const create = async () => {
    const name = newBaseName.trim();
    if (!name) {
      setError("请输入知识库名称。");
      return;
    }
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    setBusy(true);
    setError(null);
    try {
      const base = await knowledgeCreateBase({
        draft: { name, description: "", projectPath },
      });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      const rows = await knowledgeListBases({ projectPath });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      setBusy(false);
      setBases(basesForScope(rows, projectPath));
      currentViewKeyRef.current = saveViewKey(projectPath, base.id);
      viewEpochRef.current += 1;
      setBaseId(base.id);
      setCreateBase(false);
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch)) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch)) setBusy(false);
    }
  };

  const save = async () => {
    if (!baseId) {
      setError("请先选择或创建知识库。");
      return;
    }
    const requestKey = currentViewKeyRef.current;
    const requestEpoch = viewEpochRef.current;
    setBusy(true);
    setError(null);
    try {
      await knowledgeImportText({
        baseId,
        projectPath,
        draft: { title, body, sourceKind, sourceRef, verified },
      });
      if (!isCurrentView(requestKey, requestEpoch)) return;
      onSaved?.();
      onClose();
    } catch (failure) {
      if (isCurrentView(requestKey, requestEpoch)) setError(errorText(failure));
    } finally {
      if (isCurrentView(requestKey, requestEpoch)) setBusy(false);
    }
  };

  return (
    <div className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="knowledge-save-title">
      <button type="button" className="settings-modal-backdrop" aria-label="关闭入库窗口" disabled={busy} onClick={onClose} />
      <section className="settings-modal-panel wb-modal-panel knowledge-dialog">
        <header className="modal-head">
          <h2 className="modal-title" id="knowledge-save-title">加入知识库</h2>
          <span className="spacer" />
          <button type="button" className="btn" disabled={busy} onClick={onClose} data-testid="knowledge-save-close">关闭</button>
        </header>

        <div className="skill-detail-body knowledge-dialog-body">
          {error && <p className="page-error" role="alert">{error}</p>}
          <label className="field field--wide">
            <span className="field-label">保存范围</span>
            <select
              className="select"
              autoFocus
              aria-label="保存范围"
              data-testid="knowledge-save-scope"
              value={scopeValue}
              disabled={busy}
              onChange={(event) => {
                const nextScope = event.target.value;
                const nextPath = nextScope === PERSONAL ? null : nextScope;
                scopePathRef.current = nextPath;
                currentViewKeyRef.current = saveViewKey(nextPath, "");
                viewEpochRef.current += 1;
                baseListRequestRef.current += 1;
                setBases([]);
                setBaseId("");
                setScopeValue(nextScope);
                setError(null);
              }}
            >
              <option value={PERSONAL}>个人资料</option>
              {projects.map((project) => (
                <option key={project.path} value={project.path}>{project.name}</option>
              ))}
            </select>
          </label>

          <label className="field field--wide">
            <span className="field-label">目标知识库</span>
            <select
              className="select"
              aria-label="目标知识库"
              data-testid="knowledge-save-base"
              value={baseId}
              disabled={busy || bases.length === 0}
              onChange={(event) => {
                currentViewKeyRef.current = saveViewKey(projectPath, event.target.value);
                viewEpochRef.current += 1;
                setBaseId(event.target.value);
                setError(null);
              }}
            >
              {bases.length === 0 && <option value="">当前范围还没有知识库</option>}
              {bases.map((base) => <option key={base.id} value={base.id}>{base.name}</option>)}
            </select>
          </label>

          {bases.length === 0 && !createBase && (
            <div className="knowledge-inline-create">
              <p className="app-hint">先在当前范围创建一个知识库，再保存这份文本。</p>
              <button type="button" className="btn" disabled={busy} onClick={() => setCreateBase(true)}>新建知识库</button>
            </div>
          )}
          {createBase && (
            <div className="knowledge-inline-create">
              <label className="field field--wide">
                <span className="field-label">知识库名称</span>
                <input
                  className="input"
                  autoFocus
                  value={newBaseName}
                  aria-label="新知识库名称"
                  data-testid="knowledge-save-new-base-name"
                  disabled={busy}
                  onChange={(event) => { setNewBaseName(event.target.value); setError(null); }}
                />
              </label>
              <button type="button" className="btn" disabled={busy} onClick={() => setCreateBase(false)}>取消</button>
              <button type="button" className="btn btn--primary" disabled={busy} onClick={() => void create()}>
                {busy ? "创建中…" : "创建知识库"}
              </button>
            </div>
          )}

          <section className="knowledge-save-preview" aria-label="待保存正文">
            <div className="knowledge-save-preview-head">
              <strong>{title}</strong>
              <span>{body.length.toLocaleString()} 字符</span>
            </div>
            <pre>{body.slice(0, 1800)}{body.length > 1800 ? "\n…" : ""}</pre>
          </section>

          <label className="knowledge-check">
            <input
              type="checkbox"
              checked={verified}
              disabled={busy}
              onChange={(event) => { setVerified(event.target.checked); setError(null); }}
              data-testid="knowledge-save-verified"
            />
            <span>我已核实这份内容</span>
          </label>
          <p className="app-hint">
            来源：{sourceKind === "answer" ? "会话回答" : "成果中心文本"}。未勾选时会标记为“待核实”。
          </p>
        </div>

        <footer className="skill-detail-foot">
          <span className="spacer" />
          <button type="button" className="btn" disabled={busy} onClick={onClose}>取消</button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy || !body.trim() || !title.trim() || !baseId}
            data-testid="knowledge-save-confirm"
            onClick={() => void save()}
          >
            {busy ? "保存中…" : "确认加入"}
          </button>
        </footer>
      </section>
    </div>
  );
}
