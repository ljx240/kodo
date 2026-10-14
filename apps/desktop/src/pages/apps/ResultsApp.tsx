/**
 * 成果中心 — the workbench's third app.
 *
 * Two kinds of record: a text snapshot the user saved, and a reference to a
 * file on disk. A reference stores the size and mtime captured when it was
 * registered, so the row can say whether the file is still there and whether
 * it changed since — the store recomputes that on every read
 * (`ArtifactFileStatus`).
 *
 * Removing a record never touches the file, and the OS hand-off goes through
 * a record id rather than a path: `open_result_file` resolves the path from
 * the row, so the webview can never point the system opener at a file the
 * user did not register.
 */

import {
  Archive,
  Copy,
  ExternalLink,
  FileText,
  FolderOpen,
  Pencil,
  Star,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  createResult,
  deleteResult,
  listResults,
  openResultFile,
  pickFiles,
  revealResultFile,
  setResultFlags,
  updateResult,
  type AppManifestDto,
  type ArtifactDto,
  type ArtifactViewDto,
} from "../../api";
import { T } from "../../i18n";
import { workbenchErrorText } from "./errors";

const W = T.workbench.results;

type Kind = "text" | "file";

type EditorState = {
  draft: ArtifactDto;
  isNew: boolean;
  error: string | null;
};

type Props = {
  manifest: AppManifestDto;
  onBack: () => void;
};

function blankResult(): ArtifactDto {
  return {
    id: "",
    title: "",
    kind: "text",
    body: "",
    filePath: "",
    fileSize: null,
    fileModifiedAt: null,
    source: "manual",
    category: "",
    favorite: false,
    archived: false,
    version: 0,
    createdAt: 0,
    updatedAt: 0,
  };
}

function statusLabel(status: ArtifactViewDto["fileStatus"]): string | null {
  switch (status) {
    case "ok":
      return W.fileOk;
    case "missing":
      return W.fileMissing;
    case "changed":
      return W.fileChanged;
    default:
      return null;
  }
}

export function ResultsApp({ manifest, onBack }: Props) {
  const [rows, setRows] = useState<ArtifactViewDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<"all" | "favorites">("all");
  const [showArchived, setShowArchived] = useState(false);
  const [openId, setOpenId] = useState<string | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  const reload = useCallback(async (): Promise<ArtifactViewDto[]> => {
    const list = await listResults(true);
    setRows(list);
    return list;
  }, []);

  useEffect(() => {
    reload().then(
      () => setError(null),
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  }, [reload]);

  const put = useCallback((next: ArtifactDto) => {
    setRows((list) =>
      (list ?? []).map((row) =>
        row.artifact.id === next.id
          ? // The fingerprint is the store's to recompute; keep the one we have
            // until the next read rather than inventing a status here.
            { ...row, artifact: next }
          : row,
      ),
    );
  }, []);

  const fail = useCallback(
    (failure: unknown) => {
      setError(workbenchErrorText(failure));
      void reload().catch(() => {});
    },
    [reload],
  );

  const visible = useMemo(() => {
    const live = (rows ?? []).filter((row) => showArchived || !row.artifact.archived);
    return filter === "favorites" ? live.filter((row) => row.artifact.favorite) : live;
  }, [rows, filter, showArchived]);

  const archived = useMemo(() => (rows ?? []).filter((row) => row.artifact.archived), [rows]);

  const toggleFavorite = (row: ArtifactViewDto) => {
    void setResultFlags(row.artifact.id, row.artifact.version, {
      favorite: !row.artifact.favorite,
    })
      .then((next) => {
        put(next);
        setError(null);
      })
      .catch(fail);
  };

  const toggleArchived = (row: ArtifactViewDto) => {
    void setResultFlags(row.artifact.id, row.artifact.version, {
      archived: !row.artifact.archived,
    })
      .then((next) => {
        put(next);
        setError(null);
        setConfirmId(null);
      })
      .catch(fail);
  };

  const remove = (row: ArtifactViewDto) => {
    void deleteResult(row.artifact.id, row.artifact.version)
      .then(() => {
        setRows((list) => (list ?? []).filter((entry) => entry.artifact.id !== row.artifact.id));
        setError(null);
        setConfirmId(null);
        // The file itself is untouched — say so rather than implying a delete.
      })
      .catch(fail);
  };

  const copyPath = (path: string) => {
    // The clipboard needs a secure context; say so instead of failing mutely.
    if (!navigator.clipboard) {
      setError(T.workbench.errors.unknown);
      return;
    }
    void navigator.clipboard.writeText(path).then(
      () => {
        setCopied(path);
        window.setTimeout(() => setCopied((current) => (current === path ? null : current)), 1400);
      },
      () => setError(T.workbench.errors.unknown),
    );
  };

  const save = useCallback(
    (state: EditorState) => {
      const draft = state.draft;
      const title = draft.title.trim();
      if (!title) {
        setEditor((current) => (current ? { ...current, error: W.titleRequired } : current));
        return;
      }
      const body = (draft.body ?? "").trim();
      const path = (draft.filePath ?? "").trim();
      if (draft.kind === "text" && !body) {
        setEditor((current) => (current ? { ...current, error: W.bodyRequired } : current));
        return;
      }
      if (draft.kind === "file" && !path) {
        setEditor((current) => (current ? { ...current, error: W.pathRequired } : current));
        return;
      }
      const payload = {
        ...draft,
        title,
        body: draft.kind === "text" ? body : "",
        filePath: draft.kind === "file" ? path : "",
      };
      const request = state.isNew
        ? createResult({
            title,
            kind: draft.kind,
            body: payload.body,
            filePath: payload.filePath,
            category: draft.category,
          })
        : updateResult(draft.version, payload);
      request
        .then(() => {
          setError(null);
          setEditor(null);
          void reload().catch(() => {});
        })
        .catch((failure: unknown) => {
          const code = workbenchErrorText(failure);
          setEditor((current) => (current ? { ...current, error: code } : current));
          if (!state.isNew) {
            void reload()
              .then((fresh) => {
                const stored = fresh.find((row) => row.artifact.id === draft.id);
                if (!stored) return;
                setEditor((current) =>
                  current
                    ? { ...current, draft: { ...current.draft, version: stored.artifact.version } }
                    : current,
                );
              })
              .catch(() => {});
          }
        });
    },
    [reload],
  );

  const pick = () => {
    void pickFiles().then((paths) => {
      const first = paths?.[0];
      if (!first) return;
      setEditor((current) =>
        current
          ? {
              ...current,
              draft: {
                ...current.draft,
                kind: "file",
                filePath: first,
                title: current.draft.title || first.split("/").pop() || first,
              },
            }
          : current,
      );
    });
  };

  const detail = (row: ArtifactViewDto) => {
    const artifact = row.artifact;
    const status = statusLabel(row.fileStatus);
    return (
      <div className="result-detail" data-testid={`result-detail-${artifact.id}`}>
        {artifact.kind === "text" ? (
          <pre className="result-body">{artifact.body}</pre>
        ) : (
          <div className="result-file">
            <p className="result-path">{artifact.filePath}</p>
            <div className="result-file-meta">
              {status && (
                <span
                  className={`result-status result-status--${row.fileStatus}`}
                  data-testid={`result-status-${artifact.id}`}
                >
                  {status}
                </span>
              )}
              {row.currentSize !== null && <span className="result-size">{row.currentSize} B</span>}
              <span className="spacer" />
              <button
                type="button"
                className="btn"
                data-testid={`result-open-${artifact.id}`}
                onClick={() => void openResultFile(artifact.id).catch(fail)}
              >
                <ExternalLink size={13} strokeWidth={1.8} />
                {W.openFile}
              </button>
              <button
                type="button"
                className="btn"
                data-testid={`result-reveal-${artifact.id}`}
                onClick={() => void revealResultFile(artifact.id).catch(fail)}
              >
                <FolderOpen size={13} strokeWidth={1.8} />
                {W.revealFile}
              </button>
              <button
                type="button"
                className="btn"
                data-testid={`result-copy-${artifact.id}`}
                onClick={() => copyPath(artifact.filePath ?? "")}
              >
                <Copy size={13} strokeWidth={1.8} />
                {copied === artifact.filePath ? W.copied : W.copyPath}
              </button>
            </div>
            {row.fileStatus === "missing" && <p className="app-hint">{W.missingHint}</p>}
            {row.fileStatus === "changed" && <p className="app-hint">{W.changedHint}</p>}
          </div>
        )}
        <p className="result-provenance">
          {W.source} ·{" "}
          {artifact.source === "ai"
            ? W.sources.ai
            : artifact.source === "scheduled"
              ? W.sources.scheduled
              : W.sources.manual}
          {artifact.category && ` · ${W.category} ${artifact.category}`}
          {artifact.archived && ` · ${W.archived}`}
        </p>
      </div>
    );
  };

  return (
    <main className="main page-app">
      <header className="page-head">
        <div className="page-head-titles">
          <h1>{manifest.name}</h1>
          <p className="page-subtitle">{manifest.description}</p>
        </div>
        <div className="page-head-actions">
          <button type="button" className="btn" onClick={onBack} data-testid="app-back">
            {T.workbench.back}
          </button>
        </div>
      </header>

      <div className="app-body">
        {error && (
          <p className="page-error" role="alert">
            {error}
          </p>
        )}

        <div className="app-toolbar">
          <div className="app-filters" role="group" aria-label={W.filterLabel}>
            {(["all", "favorites"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className={`app-filter${filter === value ? " app-filter--active" : ""}`}
                aria-pressed={filter === value}
                onClick={() => setFilter(value)}
                data-testid={`result-filter-${value}`}
              >
                {value === "all" ? W.all : W.favorites}
              </button>
            ))}
          </div>

          <button
            type="button"
            className={`app-filter${showArchived ? " app-filter--active" : ""}`}
            aria-pressed={showArchived}
            onClick={() => setShowArchived((on) => !on)}
            data-testid="result-archived-toggle"
          >
            {W.archived}
          </button>

          <span className="spacer" />

          <button
            type="button"
            className="btn btn--primary"
            data-testid="result-new"
            onClick={() => setEditor({ draft: blankResult(), isNew: true, error: null })}
          >
            {W.new}
          </button>
        </div>

        {(rows ?? []).length === 0 && <p className="page-empty">{W.empty}</p>}
        {(rows ?? []).length > 0 && visible.length === 0 && (
          <p className="page-empty">{W.emptyFiltered}</p>
        )}

        <ul className="result-list">
          {visible.map((row) => {
            const artifact = row.artifact;
            const expanded = openId === artifact.id;
            return (
              <li key={artifact.id} className="result-row" data-testid={`result-${artifact.id}`}>
                <div className="result-head">
                  <button
                    type="button"
                    className={`icon-btn icon-btn--sm${artifact.favorite ? " icon-btn--active" : ""}`}
                    title={artifact.favorite ? W.unfavorite : W.favorite}
                    aria-label={artifact.favorite ? W.unfavorite : W.favorite}
                    data-testid={`result-favorite-${artifact.id}`}
                    onClick={() => toggleFavorite(row)}
                  >
                    <Star size={14} strokeWidth={1.8} />
                  </button>
                  <FileText size={14} strokeWidth={1.8} />
                  <button
                    type="button"
                    className="result-title"
                    data-testid={`result-toggle-${artifact.id}`}
                    onClick={() => setOpenId(expanded ? null : artifact.id)}
                  >
                    {artifact.title}
                  </button>
                  <span className={`result-kind result-kind--${artifact.kind}`}>
                    {artifact.kind === "text" ? W.kinds.text : W.kinds.file}
                  </span>
                  {artifact.category && (
                    <span className="result-category">{artifact.category}</span>
                  )}
                  {artifact.kind === "file" && statusLabel(row.fileStatus) && (
                    <span
                      className={`result-status result-status--${row.fileStatus}`}
                      data-testid={`result-status-${artifact.id}`}
                    >
                      {statusLabel(row.fileStatus)}
                    </span>
                  )}
                  <span className="spacer" />
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm"
                    title={W.edit}
                    aria-label={W.edit}
                    data-testid={`result-edit-${artifact.id}`}
                    onClick={() => setEditor({ draft: { ...artifact }, isNew: false, error: null })}
                  >
                    <Pencil size={13} strokeWidth={1.8} />
                  </button>
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm"
                    title={artifact.archived ? W.unarchive : W.archive}
                    aria-label={artifact.archived ? W.unarchive : W.archive}
                    data-testid={`result-archive-${artifact.id}`}
                    onClick={() => toggleArchived(row)}
                  >
                    <Archive size={13} strokeWidth={1.8} />
                  </button>
                  <button
                    type="button"
                    className={`icon-btn icon-btn--sm${confirmId === artifact.id ? " icon-btn--danger" : ""}`}
                    title={confirmId === artifact.id ? W.deleteConfirm : W.delete}
                    aria-label={confirmId === artifact.id ? W.deleteConfirm : W.delete}
                    data-testid={`result-delete-${artifact.id}`}
                    onClick={() =>
                      confirmId === artifact.id ? remove(row) : setConfirmId(artifact.id)
                    }
                  >
                    <Trash2 size={13} strokeWidth={1.8} />
                  </button>
                </div>
                {confirmId === artifact.id && (
                  <p className="app-hint" role="alert">
                    {W.deleteHint}
                  </p>
                )}
                {expanded && detail(row)}
              </li>
            );
          })}
        </ul>

        {showArchived && archived.length > 0 && (
          <section className="result-archived" data-testid="result-archived">
            <h2 className="app-section-title">{W.archived}</h2>
            <ul className="result-list">
              {archived.map((row) => (
                <li key={row.artifact.id} className="result-row result-row--archived">
                  <div className="result-head">
                    <span className="result-title">{row.artifact.title}</span>
                    <span className="spacer" />
                    <button
                      type="button"
                      className="btn"
                      data-testid={`result-unarchive-${row.artifact.id}`}
                      onClick={() => toggleArchived(row)}
                    >
                      {W.unarchive}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

      {editor && (
        <div
          className="settings-modal"
          role="dialog"
          aria-modal="true"
          aria-label={editor.isNew ? W.new : W.edit}
        >
          <div className="settings-modal-backdrop" onClick={() => setEditor(null)} />
          <div className="settings-modal-panel wb-modal-panel">
            <header className="modal-head">
              <span className="modal-title">{editor.isNew ? W.new : W.edit}</span>
              <span className="spacer" />
              <button
                type="button"
                className="icon-btn icon-btn--sm"
                title={T.action.close}
                aria-label={T.action.close}
                onClick={() => setEditor(null)}
              >
                <X size={15} strokeWidth={1.8} />
              </button>
            </header>

            <div className="skill-detail-body skill-detail-body--edit">
              {editor.error && (
                <p className="page-error" role="alert">
                  {editor.error}
                </p>
              )}

              <label className="field field--wide">
                <span className="field-label">{W.titleLabel}</span>
                <input
                  className="input"
                  autoFocus
                  value={editor.draft.title}
                  aria-label={W.titleLabel}
                  data-testid="result-edit-title"
                  onChange={(event) =>
                    setEditor((current) =>
                      current
                        ? { ...current, draft: { ...current.draft, title: event.target.value } }
                        : current,
                    )
                  }
                />
              </label>

              <div className="app-form-row">
                <label className="field">
                  <span className="field-label">{W.kindLabel}</span>
                  <select
                    className="select"
                    value={editor.draft.kind}
                    aria-label={W.kindLabel}
                    data-testid="result-edit-kind"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? {
                              ...current,
                              draft: { ...current.draft, kind: event.target.value as Kind },
                            }
                          : current,
                      )
                    }
                  >
                    <option value="text">{W.kinds.text}</option>
                    <option value="file">{W.kinds.file}</option>
                  </select>
                </label>

                <label className="field">
                  <span className="field-label">{W.category}</span>
                  <input
                    className="input"
                    value={editor.draft.category}
                    aria-label={W.category}
                    data-testid="result-edit-category"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? {
                              ...current,
                              draft: { ...current.draft, category: event.target.value },
                            }
                          : current,
                      )
                    }
                  />
                </label>
              </div>

              {editor.draft.kind === "text" ? (
                <label className="field field--wide">
                  <span className="field-label">{W.bodyLabel}</span>
                  <textarea
                    className="input todo-notes"
                    rows={10}
                    value={editor.draft.body ?? ""}
                    aria-label={W.bodyLabel}
                    data-testid="result-edit-body"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? { ...current, draft: { ...current.draft, body: event.target.value } }
                          : current,
                      )
                    }
                  />
                </label>
              ) : (
                <label className="field field--wide">
                  <span className="field-label">{W.filePathLabel}</span>
                  <div className="result-path-row">
                    <input
                      className="input"
                      value={editor.draft.filePath ?? ""}
                      aria-label={W.filePathLabel}
                      data-testid="result-edit-path"
                      onChange={(event) =>
                        setEditor((current) =>
                          current
                            ? {
                                ...current,
                                draft: { ...current.draft, filePath: event.target.value },
                              }
                            : current,
                        )
                      }
                    />
                    <button
                      type="button"
                      className="btn"
                      onClick={pick}
                      data-testid="result-edit-pick"
                    >
                      <FolderOpen size={13} strokeWidth={1.8} />
                      {W.openFile}
                    </button>
                  </div>
                </label>
              )}

              {editor.draft.source !== "manual" && (
                <p className="app-hint">
                  {W.source} · {editor.draft.source === "ai" ? W.sources.ai : W.sources.scheduled}
                </p>
              )}
            </div>

            <footer className="skill-detail-foot">
              <span className="spacer" />
              <button type="button" className="btn" onClick={() => setEditor(null)}>
                {W.cancel}
              </button>
              <button
                type="button"
                className="btn btn--primary"
                data-testid="result-edit-save"
                onClick={() => save(editor)}
              >
                {W.save}
              </button>
            </footer>
          </div>
        </div>
      )}
    </main>
  );
}
