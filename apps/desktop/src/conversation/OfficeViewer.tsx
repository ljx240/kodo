import { ChevronLeft, ChevronRight, ExternalLink, FolderOpen } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  closeOffice, listOfficeDrafts, openExternalOffice, openOffice, pickOffice,
  readOffice, renderOffice, saveOffice, type OfficeDraft, type OfficeItem,
  type OfficePanelTarget, type OfficePreview, type OfficeSaved, type OfficeSourceTarget,
  type OpenedOffice,
} from "./officeApi";
import "./office-viewer.css";

type OfficeDocumentProps = {
  source: OpenedOffice;
  draft?: never;
} | {
  source?: never;
  draft: OfficeDraft;
  saved?: OfficeSaved;
  onSaved: (result: OfficeSaved) => void;
  onDiscard: () => void;
};

function OfficeDocument(props: OfficeDocumentProps) {
  const { source, draft } = props;
  const document = draft ?? source;
  const format = draft?.format ?? source?.view.format;
  const [mode, setMode] = useState<"pages" | "structure">(format === "xlsx" ? "structure" : "pages");
  const [reviewed, setReviewed] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const saved = draft ? props.saved : undefined;
  const draftOnly = Boolean(draft && (draft.needsCalculation || (draft.needsPreview && !reviewed)));

  async function perform(action: () => Promise<void>) {
    setError("");
    setWorking(true);
    try {
      await action();
    } catch (failure) {
      setError(String(failure));
    } finally {
      setWorking(false);
    }
  }

  return (
    <section className="office-document" aria-label="Office 文件预览" data-testid="office-viewer">
      <header className="office-header">
        <h3 title={source?.sourcePath ?? document.name}>{document.name}</h3>
        {source && (
          <button className="btn btn--sm" disabled={working} onClick={() => void perform(() => openExternalOffice(source.handle))}>
            <ExternalLink size={13} /> 系统打开
          </button>
        )}
        {draft && (
          <button className="btn btn--sm" disabled={working || Boolean(saved && !saved.registrationError)}
            onClick={() => void perform(async () => {
              const result = await saveOffice(draft.handle, draftOnly);
              if (result) props.onSaved(result);
            })}>
            {saved?.registrationError ? (saved.resultId ? "重试保存" : "恢复保存记录") : draftOnly ? "另存为草稿" : "另存成果"}
          </button>
        )}
      </header>
      <div className="office-actions office-view-tabs" role="group" aria-label="预览方式">
        <button className="btn btn--sm" aria-pressed={mode === "structure"} onClick={() => setMode("structure")}>
          {format === "xlsx" ? "工作表" : "结构"}
        </button>
        <button className="btn btn--sm" aria-pressed={mode === "pages"} onClick={() => setMode("pages")}>
          {format === "xlsx" ? "打印版式" : format === "pptx" ? "幻灯片" : "页面"}
        </button>
        {draft && !saved && (
          <button className="btn btn--sm office-discard" disabled={working}
            onClick={() => void perform(async () => {
              await closeOffice(draft.handle);
              props.onDiscard();
            })}>放弃草稿</button>
        )}
      </div>
      {error && <p className="office-status" role="alert">{error}</p>}
      {saved ? (
        <p className="office-status" role="status">
          {saved.resultId ? "已登记成果" : "文件已保存"} {saved.path}
          {saved.registrationError && <span>。登记未完成，{saved.registrationError}</span>}
        </p>
      ) : draft?.needsCalculation ? (
        <p className="office-status" role="status">部分公式或图表需要在 Excel 中复算。</p>
      ) : draftOnly ? (
        <p className="office-status" role="status">查看版式后可另存为成果。</p>
      ) : null}
      {mode === "pages" ? (
        <OfficePages handle={document.handle} draft={Boolean(draft)} version={source?.view.version} onReviewed={setReviewed} />
      ) : (
        <OfficeStructure handle={document.handle} draft={Boolean(draft)} initialView={source?.view} />
      )}
      {draft && draft.changes.length > 0 && (
        <details className="office-diff">
          <summary>变更明细（{draft.changes.length}）</summary>
          <ol>
            {draft.changes.map((change, index) => (
              <li key={index}>
                <strong>{change.section} {change.item}</strong>
                <dl>
                  <div><dt>修改前</dt><dd>{change.before || "空"}</dd></div>
                  <div><dt>修改后</dt><dd>{change.after || "空"}</dd></div>
                </dl>
              </li>
            ))}
          </ol>
        </details>
      )}
    </section>
  );
}

function OfficePages({ handle, draft, version, onReviewed }: {
  handle: string;
  draft: boolean;
  version?: string;
  onReviewed: (reviewed: boolean) => void;
}) {
  const [page, setPage] = useState(1);
  const [preview, setPreview] = useState<{ page: number; result: OfficePreview }>();
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    setError("");
    void renderOffice(handle, draft, page).then((result) => {
      if (!active) return;
      if (version && version !== result.version) throw new Error("文件版本已变化，请重新打开");
      setPreview({ page, result });
      onReviewed(true);
    }).catch((failure) => { if (active) setError(String(failure)); });
    return () => { active = false; };
  }, [handle, draft, version, page, onReviewed]);

  const loading = !error && preview?.page !== page;
  return (
    <section className="office-pages" aria-label="版式预览" aria-busy={loading}>
      {preview && (
        <div className="office-actions office-pager">
          <button className="btn btn--sm" aria-label="上一页" disabled={loading || page === 1} onClick={() => setPage(page - 1)}>
            <ChevronLeft size={15} />
          </button>
          <span>{page} / {preview.result.pages}</span>
          <button className="btn btn--sm" aria-label="下一页" disabled={loading || page >= preview.result.pages} onClick={() => setPage(page + 1)}>
            <ChevronRight size={15} />
          </button>
        </div>
      )}
      {error && <p className="office-status" role="alert">{error}</p>}
      {loading && <p className="office-status" role="status">正在渲染预览…</p>}
      {preview?.page === page && <img src={preview.result.image} alt={"第 " + page + " 页"} />}
    </section>
  );
}

function OfficeStructure({ handle, draft, initialView }: {
  handle: string;
  draft: boolean;
  initialView?: OpenedOffice["view"];
}) {
  const [view, setView] = useState(initialView);
  const [range, setRange] = useState("");
  const [appliedRange, setAppliedRange] = useState("");
  const [busy, setBusy] = useState(!initialView);
  const [error, setError] = useState("");

  useEffect(() => {
    if (initialView) return;
    let active = true;
    void readOffice(handle, draft, null, 0, "").then((next) => {
      if (active) setView(next);
    }).catch((failure) => {
      if (active) setError(String(failure));
    }).finally(() => {
      if (active) setBusy(false);
    });
    return () => { active = false; };
  }, [handle, draft, initialView]);

  async function load(section: string, offset: number, selectedRange: string) {
    setError("");
    setBusy(true);
    try {
      setView(await readOffice(handle, draft, section, offset, selectedRange));
      setAppliedRange(selectedRange);
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="office-structure" aria-busy={busy}>
      {error && <p className="office-status" role="alert">{error}</p>}
      {!view && busy && <p className="office-status" role="status">正在读取文件…</p>}
      {view && (
        <>
          {view.readOnly && <p className="office-status">{view.readOnly}</p>}
          <div className="office-actions">
            <label>
              {view.format === "xlsx" ? "工作表" : "位置"}
              <select value={view.section} disabled={busy} onChange={(event) => {
                setRange("");
                void load(event.target.value, 0, "");
              }}>
                {view.sections.map((section) => (
                  <option key={section.id} value={section.id}>
                    {section.label}{section.kind === "hidden_sheet" ? "（隐藏）" : ""}
                  </option>
                ))}
              </select>
            </label>
            {view.format === "xlsx" && (
              <form className="office-actions" onSubmit={(event) => {
                event.preventDefault();
                void load(view.section, 0, range.trim());
              }}>
                <label>范围 <input value={range} onChange={(event) => setRange(event.target.value)} placeholder="A1:F20" /></label>
                <button className="btn btn--sm" disabled={busy}>查看</button>
              </form>
            )}
          </div>
          {view.format === "xlsx" && <WorksheetGrid items={view.items} />}
          <ul className="office-objects">
            {view.items.filter((item) => !item.cell).map((item) => (
              <li key={item.id}>
                <strong>{item.label}</strong>
                {item.text && <span>{item.text}{item.clipped ? "…" : ""}</span>}
                {item.chart && (
                  <ul>
                    {item.chart.categories.map((category, index) => (
                      <li key={index}>{category}　{item.chart?.values[index]}</li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
          {view.items.length === 0 && <p className="office-status">这个范围没有内容。</p>}
          {view.total > 0 && (
            <div className="office-actions office-pager">
              <button className="btn btn--sm" disabled={busy || view.offset === 0}
                onClick={() => void load(view.section, 0, appliedRange)}>回到起始位置</button>
              <span>{view.offset + 1}–{view.offset + view.items.length} / {view.total}</span>
              {view.nextOffset !== null && (
                <button className="btn btn--sm" disabled={busy}
                  onClick={() => void load(view.section, view.nextOffset!, appliedRange)}>继续查看</button>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function WorksheetGrid({ items }: { items: OfficeItem[] }) {
  const rows = new Map<string, Map<string, OfficeItem>>();
  const columns = new Set<string>();
  for (const item of items) {
    if (!item.cell) continue;
    const [, column, row] = /^([A-Z]+)([0-9]+)$/.exec(item.id)!;
    let cells = rows.get(row);
    if (!cells) rows.set(row, (cells = new Map()));
    cells.set(column, item);
    columns.add(column);
  }
  const ordered = [...columns].sort((a, b) => a.length - b.length || a.localeCompare(b));
  if (!ordered.length) return null;
  return (
    <div className="office-grid">
      <table aria-label="工作表网格">
        <thead><tr><th scope="col">行</th>{ordered.map((column) => <th scope="col" key={column}>{column}</th>)}</tr></thead>
        <tbody>{[...rows].map(([row, cells]) => (
          <tr key={row}>
            <th scope="row">{row}</th>
            {ordered.map((column) => {
              const item = cells.get(column);
              const formula = item?.cell?.formula;
              return (
                <td key={column} title={item?.cell?.numberFormat ?? undefined}>
                  {item?.text}{item?.clipped ? "…" : ""}
                  {formula && (
                    <small className="office-formula">
                      ={formula}{item.cell?.cache === "cached" ? "（缓存值）" : "（待复算）"}
                    </small>
                  )}
                </td>
              );
            })}
          </tr>
        ))}</tbody>
      </table>
    </div>
  );
}

function OfficeSourcePreview({ target }: { target: OfficeSourceTarget }) {
  const [opened, setOpened] = useState<OpenedOffice>();
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    let handle: string | undefined;
    void openOffice(target).then((file) => {
      if (active) {
        handle = file.handle;
        setOpened(file);
      } else void closeOffice(file.handle);
    }).catch((failure) => { if (active) setError(String(failure)); });
    return () => {
      active = false;
      if (handle) void closeOffice(handle);
    };
  }, [target]);
  return opened
    ? <OfficeDocument key={opened.handle} source={opened} />
    : <p className="office-status" role={error ? "alert" : "status"}>{error || "正在读取 Office 文件…"}</p>;
}

function OfficePicker() {
  const [opened, setOpened] = useState<OpenedOffice>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const current = useRef<string | null>(null);
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      if (current.current) void closeOffice(current.current);
    };
  }, []);

  async function pick() {
    setError("");
    setBusy(true);
    try {
      const file = await pickOffice();
      if (!file) return;
      if (active.current) {
        if (current.current) void closeOffice(current.current);
        current.current = file.handle;
        setOpened(file);
      } else await closeOffice(file.handle);
    } catch (failure) {
      if (active.current) setError(String(failure));
    } finally {
      if (active.current) setBusy(false);
    }
  }

  return (
    <>
      <div className="office-actions office-picker">
        <button className="btn btn--sm" disabled={busy} onClick={() => void pick()}>
          <FolderOpen size={14} /> 选择文件
        </button>
      </div>
      {error && <p className="office-status" role="alert">{error}</p>}
      {opened && <OfficeDocument key={opened.handle} source={opened} />}
    </>
  );
}

function OfficeDrafts({ session, turn }: { session: string; turn: number }) {
  const [drafts, setDrafts] = useState<OfficeDraft[]>();
  const [selected, setSelected] = useState("");
  const [saves, setSaves] = useState<Record<string, OfficeSaved>>({});
  const published = useRef(new Set<string>());
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    const completed = published.current;
    void listOfficeDrafts(session, turn).then((next) => {
      if (active) setDrafts(next);
    }).catch((failure) => { if (active) setError(String(failure)); });
    return () => {
      active = false;
      for (const handle of completed) void closeOffice(handle);
    };
  }, [session, turn]);

  const draft = drafts?.find((entry) => entry.handle === selected) ?? drafts?.[0];
  if (!draft) {
    return <p className="office-status" role={error ? "alert" : "status"}>
      {error || (drafts ? "此轮没有待审阅的草稿。已登记文件可在成果中心查看。" : "正在读取 Office 草稿…")}
    </p>;
  }
  return (
    <>
      {drafts && drafts.length > 1 && (
        <div className="office-actions office-picker">
          <label>草稿
            <select value={draft.handle} onChange={(event) => setSelected(event.target.value)}>
              {drafts.map((entry) => <option key={entry.handle} value={entry.handle}>{entry.name}</option>)}
            </select>
          </label>
        </div>
      )}
      <OfficeDocument key={draft.handle} draft={draft} saved={saves[draft.handle]}
        onSaved={(result) => {
          if (!result.registrationError) published.current.add(draft.handle);
          setSaves((previous) => ({ ...previous, [draft.handle]: result }));
        }}
        onDiscard={() => setDrafts((previous) => previous?.filter((entry) => entry.handle !== draft.handle))}
      />
    </>
  );
}

export function OfficeProjectViewer({ project, path }: { project: string; path: string }) {
  const target = useMemo(() => ({ kind: "project" as const, project, path }), [project, path]);
  return <OfficeSourcePreview target={target} />;
}

export function OfficeSidebar({ target }: { target: OfficePanelTarget }) {
  return (
    <div className="office-sidebar" data-testid="office-sidebar">
      {target.kind === "pick" ? <OfficePicker /> : target.kind === "drafts" ? (
        <OfficeDrafts session={target.session} turn={target.turn} />
      ) : <OfficeSourcePreview target={target} />}
    </div>
  );
}
