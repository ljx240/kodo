import { useState } from "react";
import { ChevronDown, ShieldCheck } from "lucide-react";
import type { KnowledgeBase, KnowledgeSelection, PreparedTurn } from "../knowledgeApi";
import "./knowledge-conversation.css";

type Props = {
  bases: KnowledgeBase[];
  selection: KnowledgeSelection;
  loading: boolean;
  busy: boolean;
  error: string | null;
  projectName: string;
  approval: PreparedTurn | null;
  onContinueWithoutKnowledge?: () => void;
  onToggleBase: (baseId: string, checked: boolean) => void;
  onStrictChange: (strict: boolean) => void;
  onRetry: () => void;
  onApprove: () => void;
  onCancelApproval: () => void;
};

export function KnowledgeBinding({
  bases,
  selection,
  loading,
  busy,
  error,
  projectName,
  approval,
  onContinueWithoutKnowledge,
  onToggleBase,
  onStrictChange,
  onRetry,
  onApprove,
  onCancelApproval,
}: Props) {
  const [open, setOpen] = useState(false);
  const personalBases = bases.filter((base) => base.projectPath === null);
  const projectBases = bases.filter((base) => base.projectPath !== null);

  return (
    <>
      <section className="knowledge-binding" aria-label="会话知识库" data-testid="knowledge-binding">
        <button
          type="button"
          className="knowledge-binding-trigger"
          aria-expanded={open}
          aria-controls="knowledge-binding-panel"
          data-testid="knowledge-binding-toggle"
          disabled={loading}
          onClick={() => setOpen((current) => !current)}
        >
          <ShieldCheck size={14} strokeWidth={1.8} aria-hidden="true" />
          <span>知识库</span>
          <span className="knowledge-binding-count">
            {loading ? "读取中…" : selection.baseIds.length > 0 ? `已选 ${selection.baseIds.length} 个` : "未绑定"}
          </span>
          <ChevronDown size={13} strokeWidth={1.8} aria-hidden="true" />
        </button>

        {error && (
          <div className="knowledge-binding-error" role="alert" data-testid="knowledge-binding-error">
            <span>{error}</span>
            <button type="button" className="btn btn--sm" disabled={loading || busy} onClick={onRetry}>
              重新读取
            </button>
            {onContinueWithoutKnowledge && (
              <button
                type="button"
                className="btn btn--sm"
                disabled={loading || busy}
                onClick={onContinueWithoutKnowledge}
                data-testid="knowledge-continue-without"
              >
                本轮不使用资料继续
              </button>
            )}
          </div>
        )}

        {open && (
          <div className="knowledge-binding-panel" id="knowledge-binding-panel">
            <div className="knowledge-binding-head">
              <div>
                <strong>本轮可检索的资料</strong>
                <p>只检索你明确勾选的知识库；切换对话或项目会重新读取绑定。</p>
              </div>
              {busy && <span className="knowledge-binding-status" role="status">处理中…</span>}
            </div>

            {loading ? (
              <p className="knowledge-binding-empty" role="status">正在读取当前范围的知识库…</p>
            ) : bases.length === 0 ? (
              <p className="knowledge-binding-empty">当前没有可选知识库。可先到工作台创建个人或项目知识库。</p>
            ) : (
              <fieldset className="knowledge-binding-options" disabled={busy}>
                <legend className="knowledge-sr-only">选择会话知识库</legend>
                {personalBases.length > 0 && (
                  <div className="knowledge-binding-group">
                    <span className="knowledge-binding-group-label">个人资料</span>
                    {personalBases.map((base) => (
                      <label className="knowledge-binding-option" key={base.id}>
                        <input
                          type="checkbox"
                          checked={selection.baseIds.includes(base.id)}
                          onChange={(event) => onToggleBase(base.id, event.target.checked)}
                          data-testid={`knowledge-bind-${base.id}`}
                        />
                        <span>{base.name}</span>
                      </label>
                    ))}
                  </div>
                )}
                {projectBases.length > 0 && (
                  <div className="knowledge-binding-group">
                    <span className="knowledge-binding-group-label">当前项目{projectName ? ` · ${projectName}` : ""}</span>
                    {projectBases.map((base) => (
                      <label className="knowledge-binding-option" key={base.id}>
                        <input
                          type="checkbox"
                          checked={selection.baseIds.includes(base.id)}
                          onChange={(event) => onToggleBase(base.id, event.target.checked)}
                          data-testid={`knowledge-bind-${base.id}`}
                        />
                        <span>{base.name}</span>
                      </label>
                    ))}
                  </div>
                )}
              </fieldset>
            )}

            <label className="knowledge-binding-strict">
              <input
                type="checkbox"
                checked={selection.strict}
                disabled={busy || loading || bases.length === 0}
                onChange={(event) => onStrictChange(event.target.checked)}
                data-testid="knowledge-strict"
              />
              <span>
                严格模式
                <small>未找到匹配资料时，本机 Agent 会拦截模型端点调用。</small>
              </span>
            </label>
          </div>
        )}
      </section>

      {approval && (
        <div className="knowledge-approval-backdrop" data-testid="knowledge-approval-backdrop">
          <section
            className="knowledge-approval-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="knowledge-approval-title"
            aria-describedby="knowledge-approval-copy"
            data-testid="knowledge-approval"
          >
            <header className="knowledge-approval-head">
              <div>
                <p className="knowledge-eyebrow">发送前确认</p>
                <h2 id="knowledge-approval-title">允许本轮将资料发送到模型？</h2>
              </div>
            </header>
            <p id="knowledge-approval-copy" className="knowledge-approval-copy">
              {approval.providerLabel
                ? `以下精确片段将随问题发送给 ${approval.providerLabel}。`
                : "以下精确片段将随问题发送到当前模型服务商。"}
              {approval.hits.length === 0 ? " 本轮没有检索到资料片段。" : " 请先检查完整预览，再明确批准。"}
            </p>
            <div className="knowledge-approval-hits" data-testid="knowledge-approval-hits">
              {approval.hits.map((hit, index) => (
                <article className="knowledge-approval-hit" key={`${hit.documentId}:${hit.chunkId}`}>
                  <div className="knowledge-approval-hit-head">
                    <strong>{hit.title}</strong>
                    <span>{hit.heading || "正文"} · 第 {hit.lineStart}–{hit.lineEnd} 行</span>
                  </div>
                  <pre>{hit.quote}</pre>
                  <span className="knowledge-approval-hit-number">片段 {index + 1}</span>
                </article>
              ))}
            </div>
            <footer className="knowledge-approval-actions">
              <button
                type="button"
                className="btn"
                autoFocus
                data-testid="knowledge-approval-cancel"
                onClick={onCancelApproval}
              >
                取消发送
              </button>
              <button
                type="button"
                className="btn btn--primary"
                data-testid="knowledge-approval-confirm"
                onClick={onApprove}
              >
                批准并发送
              </button>
            </footer>
          </section>
        </div>
      )}
    </>
  );
}
