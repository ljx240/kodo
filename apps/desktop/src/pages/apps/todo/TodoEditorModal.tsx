/**
 * 待办 — the edit modal.
 *
 * Two columns: a flexible main column for the editable fields and a
 * 320px-wide rail for the catalog-derived sections (assignee, tags, related,
 * attachments, reminders, activity). The modal shell is `settings-modal`
 * (house style), so the body owns its own grid.
 *
 * Sub-resources (attachments / reminders / activity) load on open, not on
 * save — the editor has to be self-sufficient the moment it appears, and
 * the rail never has to ask "did we already fetch?".
 */

import {
  Check,
  FileText,
  Link2,
  Paperclip,
  Plus,
  Send,
  X,
  Bell,
} from "lucide-react";
import { useState } from "react";
import {
  TODO_ITEM_TYPES,
  type TodoActivityDto,
  type TodoAttachmentDto,
  type TodoDto,
  type TodoPriorityDto,
  type TodoReminderDto,
  type TodoReminderKindDto,
} from "../../../api";
import { T } from "../../../i18n";
import { Menu, MenuItem } from "../../../shell/Menu";
import {
  Avatar,
  relativeTime,
  TodoCompleteButton,
  TodoStatusSelect,
  type ApplyStatus,
} from "./TodoBits";
import { isDone } from "./todoModel";
import {
  PRIORITIES,
  PRIORITY_LABEL,
  personOf,
  type Catalogs,
  type EditorState,
} from "./todoTypes";

const W = T.workbench.todo;

const MIN_REMINDER_OFFSETS = [5, 10, 15, 30, 60, 120, 1440];

const REMINDER_KIND_LABEL: Record<TodoReminderKindDto, string> = {
  dueBefore: W.reminderDueBefore,
  statusChange: W.reminderStatusChange,
};

export function TodoEditorModal({
  editor,
  todos,
  catalogs,
  onPatch,
  onChildText,
  onCommentText,
  onCommentSend,
  onAttachmentAdd,
  onAttachmentRemove,
  onReminderAdd,
  onReminderToggle,
  onReminderRemove,
  onRelatedToggle,
  onClose,
  onSave,
  onArchive,
  onCreateChild,
  onApply,
}: {
  editor: EditorState;
  todos: TodoDto[] | null;
  catalogs: Catalogs;
  onPatch: (patch: Partial<TodoDto>) => void;
  onChildText: (value: string) => void;
  onCommentText: (value: string) => void;
  onCommentSend: () => Promise<boolean>;
  onAttachmentAdd: () => Promise<boolean>;
  onAttachmentRemove: (attachment: TodoAttachmentDto) => void;
  onReminderAdd: (reminder: {
    kind: TodoReminderKindDto;
    offsetMinutes: number;
    message: string;
  }) => Promise<boolean>;
  onReminderToggle: (reminder: TodoReminderDto, enabled: boolean) => void;
  onReminderRemove: (reminder: TodoReminderDto) => void;
  onRelatedToggle: (otherId: string) => void;
  onClose: () => void;
  onSave: () => void;
  onArchive: () => void;
  onCreateChild: (title: string) => Promise<boolean>;
  onApply: ApplyStatus;
}) {
  const { draft, attachments, reminders, activity } = editor;
  const isNew = draft.id === "";
  const title = isNew ? W.newItemTitle : W.edit;
  const children = (todos ?? []).filter((row) => row.parentId === draft.id);
  const otherTodos = (todos ?? []).filter(
    (row) => row.id !== draft.id && row.parentId === null && !row.archived,
  );

  return (
    <div className="settings-modal" role="dialog" aria-modal="true" aria-label={title}>
      <div className="settings-modal-backdrop" onClick={onClose} />
      <div className="settings-modal-panel wb-modal-panel">
        <header className="modal-head">
          <span className="modal-title">{title}</span>
          <span className="spacer" />
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            title={T.action.close}
            aria-label={T.action.close}
            onClick={onClose}
          >
            <X size={15} strokeWidth={1.8} />
          </button>
        </header>

        <div className="todo-editor-body">
          <div className="todo-editor-main">
            {editor.error && (
              <p className="page-error" role="alert">
                {editor.error}
              </p>
            )}

            <label className="todo-editor-section">
              <input
                className="input todo-editor-title-input"
                value={draft.title}
                aria-label={W.titleLabel}
                placeholder={W.titleLabel}
                data-testid="todo-edit-title"
                onChange={(event) => onPatch({ title: event.target.value })}
              />
            </label>

            <div className="todo-editor-row">
              <label className="field">
                <span className="field-label">{W.status}</span>
                <select
                  className="select"
                  value={draft.status}
                  aria-label={W.status}
                  data-testid="todo-edit-status"
                  onChange={(event) => onPatch({ status: event.target.value })}
                >
                  {catalogs.statuses.map((status) => (
                    <option key={status.id} value={status.id}>
                      {status.name}
                    </option>
                  ))}
                </select>
              </label>

              <label className="field">
                <span className="field-label">{W.priority}</span>
                <select
                  className="select"
                  value={draft.priority}
                  aria-label={W.priority}
                  data-testid="todo-edit-priority"
                  onChange={(event) =>
                    onPatch({ priority: event.target.value as TodoPriorityDto })
                  }
                >
                  {PRIORITIES.map((value) => (
                    <option key={value} value={value}>
                      {PRIORITY_LABEL[value]}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <div className="todo-editor-row">
              <label className="field">
                <span className="field-label">{W.due}</span>
                <input
                  type="date"
                  className="input"
                  value={draft.dueDate ?? ""}
                  aria-label={W.due}
                  data-testid="todo-edit-due"
                  onChange={(event) => onPatch({ dueDate: event.target.value || null })}
                />
              </label>
              <label className="field">
                <span className="field-label">{W.dueTime}</span>
                <input
                  type="time"
                  className="input"
                  value={draft.dueTime ?? ""}
                  aria-label={W.dueTime}
                  data-testid="todo-edit-due-time"
                  onChange={(event) => onPatch({ dueTime: event.target.value || null })}
                />
              </label>
            </div>

            <div className="todo-editor-row">
              <label className="field">
                <span className="field-label">{W.module}</span>
                <input
                  className="input"
                  value={draft.module}
                  aria-label={W.module}
                  placeholder={W.module}
                  data-testid="todo-edit-module"
                  onChange={(event) => onPatch({ module: event.target.value })}
                />
              </label>
              <label className="field">
                <span className="field-label">{W.estimate}</span>
                <input
                  type="number"
                  min={0}
                  step={0.5}
                  className="input"
                  value={draft.estimateHours ?? ""}
                  aria-label={W.estimate}
                  data-testid="todo-edit-estimate"
                  onChange={(event) =>
                    onPatch({
                      estimateHours: event.target.value === "" ? null : Number(event.target.value),
                    })
                  }
                />
              </label>
            </div>

            <div className="todo-editor-row">
              <label className="field">
                <span className="field-label">{W.actual}</span>
                <input
                  type="number"
                  min={0}
                  step={0.5}
                  className="input"
                  value={draft.actualHours ?? ""}
                  aria-label={W.actual}
                  data-testid="todo-edit-actual"
                  onChange={(event) =>
                    onPatch({
                      actualHours: event.target.value === "" ? null : Number(event.target.value),
                    })
                  }
                />
              </label>
              <label className="field">
                <span className="field-label">{W.itemType}</span>
                <select
                  className="select"
                  value={draft.itemType}
                  aria-label={W.itemType}
                  data-testid="todo-edit-item-type"
                  onChange={(event) => onPatch({ itemType: event.target.value })}
                >
                  {TODO_ITEM_TYPES.map((value) => (
                    <option key={value} value={value}>
                      {W.itemTypes[value]}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">{W.description}</h3>
              <textarea
                className="input todo-editor-textarea todo-notes"
                value={draft.notes}
                aria-label={W.description}
                placeholder={W.description}
                data-testid="todo-edit-notes"
                onChange={(event) => onPatch({ notes: event.target.value })}
              />
            </section>

            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">{W.tagsLabel}</h3>
              {catalogs.tags.length === 0 ? (
                <span className="todo-chip-empty">{W.noTags}</span>
              ) : (
                <div className="todo-tag-picker" role="group" aria-label={W.tagsLabel}>
                  {catalogs.tags.map((tag) => {
                    const on = draft.tagIds.includes(tag.id);
                    return (
                      <button
                        key={tag.id}
                        type="button"
                        className={`todo-tag-pick${on ? " todo-tag-pick--on" : ""}`}
                        data-testid={`todo-edit-tag-${tag.id}`}
                        aria-pressed={on}
                        onClick={() => {
                          const next = on
                            ? draft.tagIds.filter((id) => id !== tag.id)
                            : [...draft.tagIds, tag.id];
                          onPatch({ tagIds: next });
                        }}
                      >
                        <span className="todo-tag" data-color={tag.color}>
                          {tag.name}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </section>

            <section className="todo-editor-children">
              <h3 className="todo-editor-section-title">{W.subtasks}</h3>
              {children.length === 0 ? null : (
                <ul className="todo-list" data-testid="todo-edit-children">
                  {children.map((child) => (
                    <li key={child.id} className="todo-list-item" data-testid={`todo-child-${child.id}`}>
                      <TodoStatusSelect todo={child} catalogs={catalogs} onApply={onApply} />
                      <span className={`todo-list-title${isDone(child) ? " todo-list-title--done" : ""}`}>
                        {child.title}
                      </span>
                      <span className="spacer" />
                      <TodoCompleteButton todo={child} catalogs={catalogs} onApply={onApply} />
                    </li>
                  ))}
                </ul>
              )}
              <form
                className="todo-inline"
                onSubmit={(event) => {
                  event.preventDefault();
                  void onCreateChild(editor.child);
                }}
              >
                <input
                  className="input"
                  value={editor.child}
                  placeholder={W.addSubtask}
                  aria-label={W.addSubtask}
                  data-testid="todo-add-subtask"
                  onChange={(event) => onChildText(event.target.value)}
                />
              </form>
            </section>
          </div>

          <aside className="todo-editor-rail">
            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">{W.assigneeLabel}</h3>
              <select
                className="select"
                value={draft.assigneeId ?? ""}
                aria-label={W.assigneeLabel}
                data-testid="todo-edit-assignee"
                onChange={(event) =>
                  onPatch({ assigneeId: event.target.value || null })
                }
              >
                <option value="">{W.noAssignee}</option>
                {catalogs.people.map((person) => (
                  <option key={person.id} value={person.id}>
                    {person.name}
                  </option>
                ))}
              </select>
            </section>

            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">{W.related}</h3>
              {draft.linkIds.length === 0 ? (
                <span className="todo-chip-empty">{W.noRelated}</span>
              ) : (
                <ul className="todo-related-list">
                  {draft.linkIds.map((id) => {
                    const other = (todos ?? []).find((row) => row.id === id);
                    if (!other) return null;
                    return (
                      <li key={id} className="todo-related-row" data-testid={`todo-related-${id}`}>
                        <Link2 size={13} strokeWidth={1.8} />
                        <span className="todo-related-title">{other.title}</span>
                        <button
                          type="button"
                          className="todo-remove-btn"
                          aria-label={W.removeRelated}
                          onClick={() => onRelatedToggle(id)}
                        >
                          <X size={13} strokeWidth={1.8} />
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
              <RelatedMenu
                draft={draft}
                otherTodos={otherTodos}
                onToggle={onRelatedToggle}
              />
            </section>

            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">
                {W.attachments(attachments.length)}
              </h3>
              {attachments.length === 0 ? (
                <span className="todo-chip-empty">{W.noAttachments}</span>
              ) : (
                <ul className="todo-attach-list">
                  {attachments.map((attachment) => (
                    <li
                      key={attachment.id}
                      className="todo-attach-row"
                      data-testid={`todo-attach-${attachment.id}`}
                    >
                      <Paperclip size={13} strokeWidth={1.8} />
                      <span className="todo-attach-name">{attachment.name}</span>
                      <span className="todo-attach-meta">{attachment.mime}</span>
                      <button
                        type="button"
                        className="todo-remove-btn"
                        aria-label={W.removeAttachment}
                        onClick={() => onAttachmentRemove(attachment)}
                      >
                        <X size={13} strokeWidth={1.8} />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <button
                type="button"
                className="todo-add-btn"
                onClick={() => void onAttachmentAdd()}
                data-testid="todo-add-attachment"
              >
                <Plus size={12} strokeWidth={2} /> {W.uploadAttachment}
              </button>
            </section>

            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">{W.reminders}</h3>
              {reminders.length === 0 ? (
                <span className="todo-chip-empty">{W.noReminders}</span>
              ) : (
                <ul className="todo-reminder-list">
                  {reminders.map((reminder) => (
                    <li
                      key={reminder.id}
                      className="todo-reminder-row"
                      data-testid={`todo-reminder-${reminder.id}`}
                    >
                      <Bell size={13} strokeWidth={1.8} />
                      <span className="todo-reminder-text">
                        {reminder.kind === "dueBefore"
                          ? `${W.reminderDueBefore} ${W.reminderMinutes(reminder.offsetMinutes ?? 0)}`
                          : W.reminderStatusChange}
                      </span>
                      <label className="todo-attach-meta" style={{ display: "flex", alignItems: "center", gap: 4 }}>
                        <input
                          type="checkbox"
                          checked={reminder.enabled}
                          aria-label={W.reminders}
                          onChange={(event) => onReminderToggle(reminder, event.target.checked)}
                        />
                      </label>
                      <button
                        type="button"
                        className="todo-remove-btn"
                        aria-label={W.removeReminder}
                        onClick={() => onReminderRemove(reminder)}
                      >
                        <X size={13} strokeWidth={1.8} />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <ReminderComposer onSubmit={async (value) => onReminderAdd(value)} />
            </section>

            <section className="todo-editor-section">
              <h3 className="todo-editor-section-title">{W.activity(activity.length)}</h3>
              <ul className="todo-activity" data-testid="todo-activity">
                {activity.length === 0 ? (
                  <li className="todo-chip-empty">{W.noActivity}</li>
                ) : (
                  activity.map((row) => (
                    <ActivityRow key={row.id} row={row} catalogs={catalogs} />
                  ))
                )}
              </ul>
              <form
                className="todo-comment"
                onSubmit={(event) => {
                  event.preventDefault();
                  void onCommentSend();
                }}
              >
                <textarea
                  className="todo-comment-input"
                  value={editor.comment}
                  placeholder={W.commentPlaceholder}
                  aria-label={W.commentPlaceholder}
                  data-testid="todo-comment-input"
                  rows={1}
                  onChange={(event) => onCommentText(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && !event.shiftKey) {
                      event.preventDefault();
                      void onCommentSend();
                    }
                  }}
                />
                <button
                  type="submit"
                  className="todo-comment-send"
                  disabled={!editor.comment.trim()}
                  data-testid="todo-comment-send"
                  aria-label={W.sendComment}
                >
                  <Send size={14} strokeWidth={1.9} />
                </button>
              </form>
            </section>
          </aside>
        </div>

        <footer className="skill-detail-foot">
          <button
            type="button"
            className="btn"
            disabled={isNew}
            onClick={onArchive}
            data-testid="todo-edit-archive"
          >
            {draft.archived ? W.unarchive : W.archive}
          </button>
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose}>
            {W.cancel}
          </button>
          <button
            type="button"
            className="btn btn--primary"
            onClick={onSave}
            data-testid="todo-edit-save"
          >
            {W.saveAndClose}
          </button>
        </footer>
      </div>
    </div>
  );
}

function ActivityRow({
  row,
  catalogs,
}: {
  row: TodoActivityDto;
  catalogs: Catalogs;
}) {
  const author = personOf(catalogs.people, row.authorId);
  return (
    <li className="todo-activity-row" data-testid={`todo-activity-row-${row.id}`}>
      {author ? <Avatar person={author} /> : <FileText size={14} strokeWidth={1.8} />}
      <div className="todo-activity-body">
        <div className="todo-activity-meta">
          <span>{author?.name ?? "系统"}</span>
          <span>·</span>
          <span>{relativeTime(row.createdAt, "2026-09-30")}</span>
        </div>
        <div className="todo-activity-text">{row.body}</div>
      </div>
    </li>
  );
}

function RelatedMenu({
  draft,
  otherTodos,
  onToggle,
}: {
  draft: TodoDto;
  otherTodos: TodoDto[];
  onToggle: (id: string) => void;
}) {
  return (
    <div className="todo-related-pick">
      <Menu
        trigger={({ toggle }) => (
          <button type="button" className="todo-add-btn" onClick={toggle}>
            <Plus size={12} strokeWidth={2} /> {W.addRelated}
          </button>
        )}
      >
        {(close) => otherTodos.length === 0 ? (
          <span className="todo-chip-empty">{W.noRelated}</span>
        ) : otherTodos.map((other) => {
          const selected = draft.linkIds.includes(other.id);
          return (
            <MenuItem
              key={other.id}
              icon={selected ? <Check size={13} /> : <Link2 size={13} />}
              label={other.title}
              selected={selected}
              testId={`todo-related-option-${other.id}`}
              onSelect={() => {
                onToggle(other.id);
                close();
              }}
            />
          );
        })}
      </Menu>
    </div>
  );
}

function ReminderComposer({
  onSubmit,
}: {
  onSubmit: (value: { kind: TodoReminderKindDto; offsetMinutes: number; message: string }) => Promise<boolean>;
}) {
  // The composer owns its own form state; the editor only needs the submitted
  // value, so the only prop that crosses the boundary is `onSubmit`.
  return <ReminderQuickAdd onSubmit={onSubmit} />;
}

function ReminderQuickAdd({
  onSubmit,
}: {
  onSubmit: (value: { kind: TodoReminderKindDto; offsetMinutes: number; message: string }) => Promise<boolean>;
}) {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<TodoReminderKindDto>("dueBefore");
  const [offset, setOffset] = useState<number>(15);
  const [message, setMessage] = useState<string>("");
  if (!open) {
    return (
      <button
        type="button"
        className="todo-add-btn"
        onClick={() => setOpen(true)}
        data-testid="todo-add-reminder"
      >
        <Plus size={12} strokeWidth={2} /> {W.addReminder}
      </button>
    );
  }
  return (
    <form
      className="todo-editor-section"
      onSubmit={async (event) => {
        event.preventDefault();
        const ok = await onSubmit({ kind, offsetMinutes: offset, message });
        if (ok) {
          setOpen(false);
          setMessage("");
        }
      }}
    >
      <select
        className="select"
        value={kind}
        onChange={(event) => setKind(event.target.value as TodoReminderKindDto)}
        aria-label={W.reminders}
      >
        <option value="dueBefore">{REMINDER_KIND_LABEL.dueBefore}</option>
        <option value="statusChange">{REMINDER_KIND_LABEL.statusChange}</option>
      </select>
      {kind === "dueBefore" && (
        <select
          className="select"
          value={offset}
          onChange={(event) => setOffset(Number(event.target.value))}
          aria-label={W.reminderDueBefore}
        >
          {MIN_REMINDER_OFFSETS.map((value) => (
            <option key={value} value={value}>
              {W.reminderMinutes(value)}
            </option>
          ))}
        </select>
      )}
      <input
        className="input"
        placeholder={W.reminders}
        value={message}
        onChange={(event) => setMessage(event.target.value)}
      />
      <div style={{ display: "flex", gap: 6 }}>
        <button
          type="submit"
          className="btn btn--primary"
          data-testid="todo-add-reminder-save"
        >
          {W.save}
        </button>
        <button type="button" className="btn" onClick={() => setOpen(false)}>
          {W.cancel}
        </button>
      </div>
    </form>
  );
}
