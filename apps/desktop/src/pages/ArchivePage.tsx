import { useEffect, useMemo, useState } from "react";
import { Archive, ArrowDownUp, MessageCircle, Search } from "lucide-react";
import { isDesktop, listArchived, type ArchivedItemDto } from "../api";
import { archiveFooter, archivedConversations } from "../data/demo";

type Row = {
  id: string;
  title: string;
  summary: string;
  archivedAt: string;
  model: string;
  added: number;
  removed: number;
  filesChanged: number;
  projectName: string;
  live: boolean;
};

function formatWhen(at: number): string {
  const date = new Date(at * 1000);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString();
}

export function ArchivePage({
  demo,
  selectedId,
  reloadToken = 0,
  onSelect,
}: {
  demo: boolean;
  selectedId: string | null;
  /** Bump after restore/archive so the table refetches. */
  reloadToken?: number;
  onSelect: (row: {
    id: string;
    title: string;
    projectName: string;
    model: string;
    at: string;
    summary: string;
  }) => void;
}) {
  const live = !demo && isDesktop();
  const [rows, setRows] = useState<Row[]>(() =>
    archivedConversations.map((item) => ({
      id: item.id,
      title: item.title,
      summary: item.summary,
      archivedAt: item.archivedAt,
      model: item.model,
      added: item.added,
      removed: item.removed,
      filesChanged: item.added + item.removed > 0 ? 7 : 0,
      projectName: "—",
      live: false,
    })),
  );
  const [total, setTotal] = useState(archiveFooter.total);
  const [query, setQuery] = useState("");
  const [reverse, setReverse] = useState(false);

  const visibleRows = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = q
      ? rows.filter(
          (row) =>
            row.title.toLowerCase().includes(q) ||
            row.summary.toLowerCase().includes(q) ||
            row.projectName.toLowerCase().includes(q),
        )
      : rows;
    if (reverse) list = [...list].reverse();
    return list;
  }, [rows, query, reverse]);

  useEffect(() => {
    if (!live) return;
    let alive = true;
    void listArchived().then((list) => {
      if (!alive) return;
      if (!list) {
        setRows([]);
        setTotal(0);
        return;
      }
      const mapped: Row[] = list.map((item: ArchivedItemDto) => ({
        id: item.id,
        title: item.title,
        summary: item.summary || item.title,
        archivedAt: formatWhen(item.at),
        model: item.model || "—",
        added: item.added,
        removed: item.removed,
        filesChanged: item.filesChanged,
        projectName: item.projectName,
        live: true,
      }));
      setRows(mapped);
      setTotal(mapped.length);
    });
    return () => {
      alive = false;
    };
  }, [live, reloadToken]);

  return (
    <main className="main">
      <header className="page-head">
        <span className="page-head-mark">
          <Archive size={18} strokeWidth={1.7} />
        </span>
        <div className="page-head-text">
          <h1>Archive</h1>
          <p>Manage your archived conversations across projects</p>
        </div>
        <span className="spacer" />
        <span className="page-meta">{total} archived conversations</span>
      </header>

      <div className="scroll">
        <div className="page-inner page-inner--wide">
          <div className="search-field">
            <Search size={15} strokeWidth={1.7} />
            <input
              placeholder="搜索归档的对话..."
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              aria-label="搜索归档的对话"
            />
          </div>

          <div className="filters">
            <button
              type="button"
              className="icon-btn icon-btn--boxed"
              aria-label="Reverse sort order"
              onClick={() => setReverse((value) => !value)}
            >
              <ArrowDownUp size={15} strokeWidth={1.7} />
            </button>
          </div>

          {visibleRows.length === 0 ? (
            <p className="empty-note">
              {live ? (query ? "没有匹配的归档对话。" : "还没有归档的对话。") : "（演示数据）"}
            </p>
          ) : (
            <table className="archive-table">
              <thead>
                <tr>
                  <th>Conversation</th>
                  <th>Summary</th>
                  <th>Archived at ↓</th>
                  <th>Model</th>
                  <th>Files changed</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {visibleRows.map((item) => (
                  <tr
                    key={item.id}
                    className={selectedId === item.id ? "row--selected" : undefined}
                    onClick={() =>
                      onSelect({
                        id: item.id,
                        title: item.title,
                        projectName: item.projectName,
                        model: item.model,
                        at: item.archivedAt,
                        summary: item.summary,
                      })
                    }
                  >
                    <td>
                      <span className="arc-title">
                        <MessageCircle size={14} strokeWidth={1.8} />
                        {item.title}
                      </span>
                    </td>
                    <td className="arc-summary">{item.summary}</td>
                    <td>{item.archivedAt}</td>
                    <td>
                      <span className="model-pill">{item.model}</span>
                    </td>
                    <td>
                      {item.filesChanged > 0 ? (
                        <>
                          <span className="delta-add">+{item.added}</span>{" "}
                          <span className="delta-del">-{item.removed}</span>
                          <span className="setting-hint"> · {item.filesChanged} files</span>
                        </>
                      ) : (
                        <span className="setting-hint">{item.filesChanged} files</span>
                      )}
                    </td>
                    <td>
                      <span className="archived-pill">Archived</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}

          <footer className="table-foot">
            <span>
              {visibleRows.length === 0 ? 0 : 1}–{visibleRows.length} of {total} conversations
            </span>
            <span className="spacer" />
            <span className="setting-hint">Restore 在 Inspector 中操作</span>
          </footer>
        </div>
      </div>
    </main>
  );
}
