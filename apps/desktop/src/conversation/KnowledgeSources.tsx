import { useEffect, useRef, useState } from "react";
import {
  knowledgeInspectDocument,
  knowledgeTurnCitations,
  type DocumentDetail,
  type KnowledgeHit,
} from "../knowledgeApi";
import "./knowledge-conversation.css";

type Props = {
  sessionId: string | null;
  projectPath: string | null;
  turnSeq: number;
  finished: boolean;
  knowledgeUsed: boolean;
};

type DetailState = "idle" | "loading" | "ready" | "error";

function knowledgeError(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  const copy: Record<string, string> = {
    not_found: "这条引用对应的资料已删除，原文无法再读取。",
    scope_denied: "当前项目范围无法访问这条资料。",
    storage_unavailable: "知识库暂时不可用，原文读取失败。",
    revision_conflict: "资料版本已变化，请重新打开引用。",
    knowledge_unavailable: "知识库暂时不可用。",
    app_disabled: "知识库功能当前已停用。",
  };
  return copy[code] ?? "来源载入失败，请稍后重试。";
}

export function KnowledgeSources({
  sessionId,
  projectPath,
  turnSeq,
  finished,
  knowledgeUsed,
}: Props) {
  const [hits, setHits] = useState<KnowledgeHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<KnowledgeHit | null>(null);
  const [detail, setDetail] = useState<DocumentDetail | null>(null);
  const [detailState, setDetailState] = useState<DetailState>("idle");
  const [detailError, setDetailError] = useState<string | null>(null);
  const requestRef = useRef(0);
  const detailRequestRef = useRef(0);
  const viewKey = `${sessionId ?? ""}\u0000${projectPath ?? "personal"}\u0000${turnSeq}`;
  const viewKeyRef = useRef(viewKey);
  viewKeyRef.current = viewKey;
  const [loadedViewKey, setLoadedViewKey] = useState<string | null>(null);

  useEffect(() => {
    const requestId = ++requestRef.current;
    detailRequestRef.current += 1;
    let live = true;
    const requestKey = viewKey;
    setLoadedViewKey(requestKey);
    setHits([]);
    setError(null);
    setLoading(false);
    setSelected(null);
    setDetail(null);
    setDetailState("idle");
    setDetailError(null);
    if (!sessionId || !finished || !knowledgeUsed) {
      return () => {
        live = false;
      };
    }

    setLoading(true);
    void knowledgeTurnCitations({ sessionId, turnSeq }).then(
      (rows) => {
        if (!live || requestRef.current !== requestId || viewKeyRef.current !== requestKey) return;
        setHits(rows);
        setLoading(false);
      },
      (failure: unknown) => {
        if (!live || requestRef.current !== requestId || viewKeyRef.current !== requestKey) return;
        setError(knowledgeError(failure));
        setLoading(false);
      },
    );
    return () => {
      live = false;
    };
  }, [sessionId, projectPath, turnSeq, finished, knowledgeUsed]);

  useEffect(() => {
    if (!selected) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        detailRequestRef.current += 1;
        setSelected(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selected]);

  const openSource = (hit: KnowledgeHit) => {
    const requestId = ++detailRequestRef.current;
    const requestKey = viewKeyRef.current;
    setSelected(hit);
    setDetail(null);
    setDetailError(null);
    setDetailState("loading");
    void knowledgeInspectDocument({ id: hit.documentId, projectPath }).then(
      (value) => {
        if (detailRequestRef.current !== requestId || viewKeyRef.current !== requestKey) return;
        setDetail(value);
        setDetailState("ready");
      },
      (failure: unknown) => {
        if (detailRequestRef.current !== requestId || viewKeyRef.current !== requestKey) return;
        setDetailError(knowledgeError(failure));
        setDetailState("error");
      },
    );
  };

  const closeSource = () => {
    detailRequestRef.current += 1;
    setSelected(null);
    setDetail(null);
    setDetailState("idle");
    setDetailError(null);
  };

  if (loadedViewKey !== viewKey) return null;
  if (!sessionId || !knowledgeUsed) return null;
  if (!loading && !error && hits.length === 0) return null;

  return (
    <>
      {knowledgeUsed && (error ? (
          <p className="knowledge-citation-error" role="alert" data-testid={`knowledge-citation-error-${turnSeq}`}>
            来源读取失败：{error}
          </p>
        ) : hits.length > 0 ? (
          <div className="knowledge-citation-list" aria-label={`第 ${turnSeq} 轮资料来源`} data-testid={`knowledge-citations-${turnSeq}`}>
            <span className="knowledge-citation-list-label">资料来源</span>
            {hits.map((hit, index) => (
              <button
                type="button"
                className="knowledge-citation-button"
                key={`${hit.documentId}:${hit.chunkId}`}
                aria-label={`打开引用 K${index + 1}：${hit.title}`}
                data-testid={`knowledge-citation-${turnSeq}-${index + 1}`}
                onClick={() => openSource(hit)}
              >
                [K{index + 1}] {hit.title}
              </button>
            ))}
          </div>
        ) : loading ? (
          <p className="knowledge-citation-error" role="status" data-testid={`knowledge-citation-loading-${turnSeq}`}>
            正在读取本轮资料来源…
          </p>
        ) : null)}

      {selected && (
        <div className="knowledge-source-backdrop" data-testid="knowledge-source-backdrop">
          <section
            className="knowledge-source-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="knowledge-source-title"
            data-testid="knowledge-source-dialog"
          >
            <header className="knowledge-source-head">
              <h2 id="knowledge-source-title">{selected.title}</h2>
              <span className="spacer" />
              <button type="button" className="btn btn--sm" autoFocus onClick={closeSource} data-testid="knowledge-source-close">
                关闭
              </button>
            </header>
            <div className="knowledge-source-body">
              <p className="knowledge-source-meta">
                <span>标题：{selected.title}</span>
                <span>章节：{selected.heading || "正文"}</span>
                <span>行号：第 {selected.lineStart}–{selected.lineEnd} 行</span>
                <span>版本：{selected.revisionId}</span>
                {detail?.document.status === "disabled" && <span>资料已停用 · 历史引用</span>}
                {detail?.document.status === "quarantined" && <span>资料已隔离</span>}
              </p>
              {detailState === "loading" && <p role="status">正在读取原文快照…</p>}
              {detailError && <p className="knowledge-source-error" role="alert">{detailError}</p>}
              {detailState === "ready" && detail && (
                <blockquote className="knowledge-source-quote" data-testid="knowledge-source-quote">
                  {selected.quote}
                </blockquote>
              )}
              {detail && detail.document.revisionId === selected.revisionId && (
                <pre className="knowledge-source-snapshot" data-testid="knowledge-source-snapshot">
                  {detail.body}
                </pre>
              )}
              {detail && detail.document.revisionId !== selected.revisionId && (
                <p className="knowledge-source-error" role="status" data-testid="knowledge-source-revision-note">
                  当前资料已更新，无法读取本轮引用对应的完整旧快照；上方保留的是本轮实际命中的原文片段。
                </p>
              )}
            </div>
          </section>
        </div>
      )}
    </>
  );
}
