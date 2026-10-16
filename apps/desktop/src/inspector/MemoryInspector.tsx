import { Brain, Search, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  memoryDisable,
  memoryList,
  memorySearch,
  type MemoryEntryDto,
} from "../api";
import { T } from "../i18n";

/** Right-sidebar Memory panel. Same visual density as TraceInspector:
 *  one row per entry, no decorative cards, no second conversation column. */
export function MemoryOverview({
  project,
}: {
  project: string | null;
}) {
  const [entries, setEntries] = useState<MemoryEntryDto[]>([]);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = async () => {
    if (!project) return;
    setLoading(true);
    setError(null);
    try {
      const list = await memoryList({ project, includeHistory: true, limit: 200 });
      setEntries(list);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project]);

  const filtered = useMemo(() => {
    if (!query.trim()) return entries;
    const needle = query.trim().toLowerCase();
    return entries.filter((entry) =>
      entry.title.toLowerCase().includes(needle) ||
      entry.abstractText.toLowerCase().includes(needle) ||
      entry.kind.toLowerCase().includes(needle) ||
      (entry.scopeProjectId ?? "").toLowerCase().includes(needle),
    );
  }, [entries, query]);

  const runSearch = async () => {
    if (!project || !query.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const snippets = await memorySearch(project, query.trim(), 50);
      // Stitch the snippet into a flattened entry shape so the table
      // stays the same. Empty body / source_uris is the documented
      // cost of the snippet view.
      setEntries(snippets.map((s) => ({
        id: s.id,
        kind: s.kind,
        status: "active",
        authority: s.authority,
        sourceTrust: "untrusted",
        title: s.title,
        abstractText: s.abstractText,
        body: null,
        contentHash: "",
        confidence: 0,
        verified: false,
        conditions: [],
        sourceUris: s.sourceUris,
        scopeProjectId: null,
        scopeRepositoryId: null,
        scopeBranch: null,
        useCount: 0,
        version: 0,
        createdAt: 0,
        updatedAt: 0,
      })));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const onDisable = async (id: string) => {
    if (!window.confirm(T.memory.confirmDisable)) return;
    try {
      await memoryDisable(id);
      await refresh();
    } catch (err) {
      setError(String(err));
    }
  };

  if (!project) {
    return (
      <div className="ins-empty">
        <Brain size={18} strokeWidth={1.7} />
        <p>请先选择一个项目。</p>
      </div>
    );
  }

  return (
    <div className="ins-overview">
      <div className="memory-toolbar">
        <div className="search-field search-field--inline">
          <Search size={14} strokeWidth={1.7} />
          <input
            value={query}
            placeholder={T.memory.searchPlaceholder}
            aria-label={T.memory.search}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void runSearch();
            }}
          />
        </div>
        <button
          className="btn"
          type="button"
          onClick={() => void refresh()}
          disabled={loading}
        >
          {T.memory.refresh}
        </button>
      </div>
      {error && <p className="setting-hint" role="alert">{error}</p>}
      {filtered.length === 0 ? (
        <p className="ins-empty-text">{query ? T.memory.noResults : T.memory.empty}</p>
      ) : (
        <ul className="memory-list">
          {filtered.map((entry) => (
            <li key={entry.id} className="memory-row" data-kind={entry.kind} data-status={entry.status}>
              <div className="memory-row-head">
                <span className="memory-row-id" title={entry.id}>{shorten(entry.id, 12)}</span>
                <span className={`memory-chip memory-chip--kind`} data-kind={entry.kind}>
                  {entry.kind}
                </span>
                <span className={`memory-chip memory-chip--status`} data-status={entry.status}>
                  {entry.status}
                </span>
                <span className="memory-row-title">{entry.title || T.memory.noTitle}</span>
                <button
                  className="btn-icon"
                  type="button"
                  aria-label={T.memory.disable}
                  title={T.memory.disable}
                  onClick={() => void onDisable(entry.id)}
                >
                  <Trash2 size={14} strokeWidth={1.7} />
                </button>
              </div>
              {entry.abstractText && (
                <p className="memory-row-abstract">{entry.abstractText}</p>
              )}
              {entry.sourceUris.length > 0 && (
                <p className="memory-row-evidence">
                  {entry.sourceUris.slice(0, 3).map((uri) => (
                    <span key={uri} className="memory-evidence-uri">{uri}</span>
                  ))}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function shorten(s: string, n: number): string {
  if (s.length <= n) return s;
  return `…${s.slice(s.length - n + 1)}`;
}
