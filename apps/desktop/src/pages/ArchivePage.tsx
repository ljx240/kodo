import { useEffect, useMemo, useState } from "react";
import { Archive, ArrowDownUp, MessageCircle, Search } from "lucide-react";
import { isDesktop, listArchived, type ArchivedItemDto } from "../api";
import type { DemoState } from "../data/demoState";
import { T } from "../i18n";

const F = T.page.archive;

type Row = {
  id: string;
  title: string;
  summary: string;
  archivedAt: string;
  model: string;
  added: number;
  removed: number;
  projectName: string;
  live: boolean;
};

function formatWhen(at: number): string {
  const date = new Date(at * 1000);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString();
}

/**
 * Sort/filter key for a row's archived time. Demo fixtures freeze display
 * strings ("Today 10:25", "Apr 28, 2024") — "Today" counts as now so the
 * relative ranges behave; dated strings parse normally.
 */
function whenKey(archivedAt: string): number {
  if (archivedAt.startsWith("Today")) return Date.now();
  const parsed = Date.parse(archivedAt);
  return Number.isNaN(parsed) ? 0 : parsed;
}

type TimeRange = "all" | "7d" | "30d" | "older";
type SortKey = "archived" | "title" | "model";

export function ArchivePage({
  demo,
  demoState,
  selectedId,
  reloadToken = 0,
  onSelect,
}: {
  /** True on `/ui-demo`; demo rows then come from `demoState`. */
  demo: boolean;
  demoState?: DemoState | null;
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
  // Demo rows carry no project of their own — the fixture's workspace project
  // is the one the reference shows for these conversations (DEMO_DATA.md).
  const demoRows = useMemo<Row[]>(
    () =>
      (demoState?.archivedConversations ?? []).map((item) => ({
        id: item.id,
        title: item.title,
        summary: item.summary,
        archivedAt: item.archivedAt,
        model: item.model,
        added: item.added,
        removed: item.removed,
        projectName: demoState?.project.name ?? "—",
        live: false,
      })),
    [demoState],
  );
  const [rows, setRows] = useState<Row[]>(demoRows);
  const [total, setTotal] = useState(demoState?.archiveFooter.total ?? 0);
  const [query, setQuery] = useState("");
  const [projectFilter, setProjectFilter] = useState("all");
  const [timeRange, setTimeRange] = useState<TimeRange>("all");
  const [modelFilter, setModelFilter] = useState("all");
  const [sortBy, setSortBy] = useState<SortKey>("archived");
  const [reverse, setReverse] = useState(false);

  useEffect(() => {
    if (demo) {
      setRows(demoRows);
      setTotal(demoState?.archiveFooter.total ?? demoRows.length);
    }
  }, [demo, demoRows, demoState]);

  const projectOptions = useMemo(() => [...new Set(rows.map((r) => r.projectName))], [rows]);
  const modelOptions = useMemo(() => [...new Set(rows.map((r) => r.model))], [rows]);

  const visibleRows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const now = Date.now();
    const day = 86_400_000;
    let list = rows.filter((row) => {
      if (
        q &&
        !row.title.toLowerCase().includes(q) &&
        !row.summary.toLowerCase().includes(q) &&
        !row.projectName.toLowerCase().includes(q)
      ) {
        return false;
      }
      if (projectFilter !== "all" && row.projectName !== projectFilter) return false;
      if (modelFilter !== "all" && row.model !== modelFilter) return false;
      if (timeRange !== "all") {
        const age = now - whenKey(row.archivedAt);
        if (timeRange === "7d" && age > 7 * day) return false;
        if (timeRange === "30d" && age > 30 * day) return false;
        if (timeRange === "older" && age <= 30 * day) return false;
      }
      return true;
    });
    list = [...list].sort((a, b) => {
      if (sortBy === "title") return a.title.localeCompare(b.title, "zh-Hans-CN");
      if (sortBy === "model") return a.model.localeCompare(b.model);
      return whenKey(a.archivedAt) - whenKey(b.archivedAt);
    });
    if (reverse) list.reverse();
    return list;
  }, [rows, query, projectFilter, timeRange, modelFilter, sortBy, reverse]);

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

  const f = T.page.archive.filters;
  const archivedArrow = sortBy === "archived" ? (reverse ? " ↑" : " ↓") : "";

  return (
    <main className="main">
      <header className="page-head">
        <span className="page-head-mark">
          <Archive size={18} strokeWidth={1.7} />
        </span>
        <div className="page-head-text">
          <h1>{F.title}</h1>
          <p>{F.subtitle}</p>
        </div>
        <span className="spacer" />
        <span className="page-meta">{F.count(total)}</span>
      </header>

      <div className="scroll">
        <div className="page-inner page-inner--wide">
          <div className="search-field">
            <Search size={15} strokeWidth={1.7} />
            <input
              placeholder={F.searchPlaceholder}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              aria-label={F.searchPlaceholder}
            />
          </div>

          {/* One compact toolbar: Project / Time range / Model / Sort by (§5c). */}
          <div className="filters">
            <label className="field">
              <span className="field-label">{f.project}</span>
              <select
                className="select"
                value={projectFilter}
                onChange={(event) => setProjectFilter(event.target.value)}
              >
                <option value="all">{f.all}</option>
                {projectOptions.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">{f.timeRange}</span>
              <select
                className="select"
                value={timeRange}
                onChange={(event) => setTimeRange(event.target.value as TimeRange)}
              >
                <option value="all">{f.allTime}</option>
                <option value="7d">{f.last7}</option>
                <option value="30d">{f.last30}</option>
                <option value="older">{f.older}</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">{f.model}</span>
              <select
                className="select"
                value={modelFilter}
                onChange={(event) => setModelFilter(event.target.value)}
              >
                <option value="all">{f.all}</option>
                {modelOptions.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">{f.sortBy}</span>
              <select
                className="select"
                value={sortBy}
                onChange={(event) => setSortBy(event.target.value as SortKey)}
              >
                <option value="archived">{f.sortArchivedAt}</option>
                <option value="title">{f.sortTitle}</option>
                <option value="model">{f.sortModel}</option>
              </select>
            </label>
            <button
              type="button"
              className="icon-btn icon-btn--boxed"
              aria-label={f.reverse}
              title={f.reverse}
              onClick={() => setReverse((value) => !value)}
            >
              <ArrowDownUp size={15} strokeWidth={1.7} />
            </button>
          </div>

          {visibleRows.length === 0 ? (
            <p className="empty-note">{live ? (query ? F.emptyFiltered : F.emptyNone) : F.demoNote}</p>
          ) : (
            <table className="archive-table">
              <thead>
                <tr>
                  <th>{F.columns.conversation}</th>
                  <th>{F.columns.summary}</th>
                  <th>{F.columns.archivedAt}{archivedArrow}</th>
                  <th>{F.columns.model}</th>
                  <th>{F.columns.files}</th>
                  <th>{F.columns.status}</th>
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
                        <span className="arc-title-text">{item.title}</span>
                      </span>
                    </td>
                    <td className="arc-summary">{item.summary}</td>
                    <td>{item.archivedAt}</td>
                    <td>
                      <span className="model-pill">{item.model}</span>
                    </td>
                    <td>
                      {item.added + item.removed > 0 ? (
                        <>
                          <span className="delta-add">+{item.added}</span>{" "}
                          <span className="delta-del">-{item.removed}</span>
                        </>
                      ) : (
                        <span className="setting-hint">—</span>
                      )}
                    </td>
                    <td>
                      <span className="archived-pill">{F.statusArchived}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}

          <footer className="table-foot">
            <span>
              {F.footer(visibleRows.length === 0 ? 0 : 1, visibleRows.length, total)}
            </span>
            <span className="spacer" />
            <span className="setting-hint">{F.restoreHint}</span>
          </footer>
        </div>
      </div>
    </main>
  );
}
