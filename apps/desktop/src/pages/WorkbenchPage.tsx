/**
 * 工作台 — the app collection, its management, and 今日概览.
 *
 * The front door of the first-level 工作台 entry: it lists the apps the user
 * has enabled, opens one *inside the workbench* (`?app=`), and keeps the
 * enable / favorite / order / integration switches. 待办 / 日历 / 成果中心
 * never become sidebar rows — the sidebar keeps one 工作台 entry while an app
 * is on screen (LAYOUT §2).
 *
 * Apps are rows, not tiles: the MCP connector catalog is the only documented
 * tiling exception (DESIGN §9), so the collection follows the same compact row
 * language as the rest of the app.
 */

import { BookOpen, CalendarDays, Files, LayoutGrid, ListTodo, Search, Settings2, Star } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  getWorkbenchPrefs,
  listApps,
  listCalendarEvents,
  listTodos,
  setAppEnabled,
  setAppFavorite,
  setAppLinks,
  setAppOrder,
  setWorkbenchPref,
  sortApps,
  type AppViewDto,
  type CalendarEventDto,
  type TodoDto,
} from "../api";
import { T } from "../i18n";
import { DEMO_TODAY } from "../data/workbench";
import { hrefTo, navigate, type AppId, type Route } from "../routes";
import { workbenchErrorText } from "./apps/errors";
import { CalendarApp } from "./apps/CalendarApp";
import { KnowledgeApp } from "./apps/KnowledgeApp";
import { ResultsApp } from "./apps/ResultsApp";
import { TodoApp } from "./apps/todo/TodoApp";
import { today as todayIso } from "./apps/time";

const W = T.workbench;

/** Manifest `icon` string → component. Unknown names fall back to the grid. */
const APP_ICONS: Record<string, typeof LayoutGrid> = {
  "list-todo": ListTodo,
  "calendar-days": CalendarDays,
  files: Files,
  "book-open": BookOpen,
};

/** Which app owns which workbench pref key. `showToday` is workbench-level. */
const SHOW_TODAY = "workbench.showToday";

const MAX_TODAY = 3;

type Props = {
  route: Route;
};

export function WorkbenchPage({ route }: Props) {
  const today = route.demo ? DEMO_TODAY : todayIso();

  const [apps, setApps] = useState<AppViewDto[] | null>(null);
  const [prefs, setPrefs] = useState<Record<string, string> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [manage, setManage] = useState(false);
  const [todayTodos, setTodayTodos] = useState<TodoDto[]>([]);
  const [todayEvents, setTodayEvents] = useState<CalendarEventDto[]>([]);

  useEffect(() => {
    Promise.all([listApps(), getWorkbenchPrefs()]).then(
      ([list, stored]) => {
        setApps(list);
        setPrefs(stored);
        setError(null);
      },
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  }, []);

  const find = useCallback(
    (appId: string): AppViewDto | undefined => apps?.find((app) => app.manifest.appId === appId),
    [apps],
  );

  const enabled = useCallback(
    (appId: string): boolean => find(appId)?.preference.enabled === true,
    [find],
  );

  /** 今日概览 reads each source's own records, and only from apps that are on. */
  const showToday = prefs?.[SHOW_TODAY] !== "false";
  const summaryApp = useCallback(
    (appId: string): boolean =>
      enabled(appId) && find(appId)?.manifest.capabilities.summary === true,
    [enabled, find],
  );
  const wantTodos = showToday && summaryApp("todo");
  const wantEvents = showToday && summaryApp("calendar");

  useEffect(() => {
    // Re-read on every return to the collection, so a todo added inside the app
    // shows up here without a manual refresh.
    if (!wantTodos) {
      setTodayTodos([]);
      return;
    }
    let live = true;
    void listTodos(false)
      .then((rows) => {
        if (!live) return;
        // `isDone` comes off the status catalog, so renaming or re-flagging a
        // status cannot make a finished todo reappear here.
        setTodayTodos(rows.filter((row) => row.dueDate === today && !row.isDone));
      })
      .catch(() => live && setTodayTodos([]));
    return () => {
      live = false;
    };
  }, [wantTodos, today, route.appId]);

  useEffect(() => {
    if (!wantEvents) {
      setTodayEvents([]);
      return;
    }
    let live = true;
    void listCalendarEvents()
      .then((rows) => {
        if (!live) return;
        setTodayEvents(rows.filter((row) => row.date === today));
      })
      .catch(() => live && setTodayEvents([]));
    return () => {
      live = false;
    };
  }, [wantEvents, today, route.appId]);

  // --- writes ---------------------------------------------------------------

  const applyOrder = (list: AppViewDto[], appId: string, delta: number) => {
    const ordered = sortApps(list).map((app) => app.manifest.appId);
    const from = ordered.indexOf(appId);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= ordered.length) return;
    [ordered[from], ordered[to]] = [ordered[to], ordered[from]];
    void setAppOrder(ordered).then(setApps, (failure: unknown) =>
      setError(workbenchErrorText(failure)),
    );
  };

  const replace = (next: AppViewDto) =>
    setApps((list) =>
      (list ?? []).map((app) => (app.manifest.appId === next.manifest.appId ? next : app)),
    );

  const toggleEnabled = (app: AppViewDto) => {
    void setAppEnabled(app.manifest.appId, !app.preference.enabled).then(
      replace,
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  };

  const toggleFavorite = (app: AppViewDto) => {
    void setAppFavorite(app.manifest.appId, !app.preference.favorite).then(
      replace,
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  };

  const toggleLink = (app: AppViewDto, link: "ai" | "scheduler") => {
    const nextAi = link === "ai" ? !app.preference.aiEnabled : app.preference.aiEnabled;
    const nextSchedule =
      link === "scheduler" ? !app.preference.scheduleEnabled : app.preference.scheduleEnabled;
    void setAppLinks(app.manifest.appId, nextAi, nextSchedule).then(replace, (failure: unknown) =>
      setError(workbenchErrorText(failure)),
    );
  };

  const toggleToday = () => {
    const next = !showToday;
    setPrefs((current) => ({ ...(current ?? {}), [SHOW_TODAY]: String(next) }));
    void setWorkbenchPref(SHOW_TODAY, String(next)).catch((failure: unknown) =>
      setError(workbenchErrorText(failure)),
    );
  };

  const openApp = (appId: string) =>
    navigate(hrefTo("workbench", route.inspectorOpen, undefined, appId as AppId));
  const backToCollection = () => navigate(hrefTo("workbench", route.inspectorOpen));

  // --- view -----------------------------------------------------------------

  const openAppView = route.appId ? find(route.appId) : undefined;
  const needle = query.trim().toLowerCase();
  const matching = useMemo(() => {
    const on = sortApps(apps ?? []).filter((app) => app.preference.enabled);
    if (!needle) return on;
    return on.filter(
      (app) =>
        app.manifest.name.toLowerCase().includes(needle) ||
        app.manifest.description.toLowerCase().includes(needle),
    );
  }, [apps, needle]);
  const favorites = matching.filter((app) => app.preference.favorite);
  const rest = matching.filter((app) => !app.preference.favorite);
  const allOff = (apps ?? []).filter((app) => app.preference.enabled).length === 0;

  if (route.appId === "todo" && openAppView?.preference.enabled) {
    return <TodoApp manifest={openAppView.manifest} demo={route.demo} onBack={backToCollection} />;
  }
  if (route.appId === "calendar" && openAppView?.preference.enabled) {
    return (
      <CalendarApp manifest={openAppView.manifest} demo={route.demo} onBack={backToCollection} />
    );
  }
  if (route.appId === "artifacts" && openAppView?.preference.enabled) {
    return <ResultsApp manifest={openAppView.manifest} onBack={backToCollection} />;
  }
  if (route.appId === "knowledge" && openAppView?.preference.enabled) {
    return <KnowledgeApp manifest={openAppView.manifest} onBack={backToCollection} />;
  }

  // An app the user opened by URL while it is disabled (or gone) falls back to
  // the collection with a hint rather than rendering a dead screen.
  const appNotice =
    route.appId && apps !== null && !openAppView?.preference.enabled
      ? openAppView
        ? W.disabledBadge
        : W.unknownApp
      : null;

  return (
    <main className="main page-workbench">
      <header className="page-head">
        <div className="page-head-titles">
          <h1>{W.title}</h1>
          <p className="page-subtitle">{W.subtitle}</p>
        </div>
        <div className="page-head-actions">
          <button
            type="button"
            className="btn"
            data-testid="workbench-manage"
            onClick={() => setManage(true)}
          >
            <Settings2 size={14} strokeWidth={1.8} />
            {W.manage}
          </button>
        </div>
      </header>

      <div className="app-body">
        {error && (
          <p className="page-error" role="alert">
            {error}
          </p>
        )}
        {appNotice && (
          <p className="page-error" role="alert" data-testid="workbench-app-notice">
            {appNotice}
          </p>
        )}

        <Collection
          favorites={favorites}
          rest={rest}
          query={query}
          onQuery={setQuery}
          showToday={showToday}
          todayTodos={todayTodos}
          todayEvents={todayEvents}
          onOpen={openApp}
          onOpenTodo={() => openApp("todo")}
          onOpenCalendar={() => openApp("calendar")}
          allOff={allOff}
        />
      </div>

      {manage && apps && (
        <Manage
          apps={apps}
          showToday={showToday}
          onClose={() => setManage(false)}
          onToggleEnabled={toggleEnabled}
          onToggleFavorite={toggleFavorite}
          onToggleLink={toggleLink}
          onToggleToday={toggleToday}
          onMove={(appId, delta) => applyOrder(apps, appId, delta)}
        />
      )}
    </main>
  );
}

// ---------------------------------------------------------------------------

function appRow(app: AppViewDto, onOpen: (appId: string) => void) {
  const Icon = APP_ICONS[app.manifest.icon] ?? LayoutGrid;
  return (
    <li
      key={app.manifest.appId}
      className="wb-app-row"
      data-testid={`wb-app-${app.manifest.appId}`}
    >
      <Icon size={16} strokeWidth={1.8} />
      <button
        type="button"
        className="wb-app-name"
        data-testid={`wb-open-${app.manifest.appId}`}
        onClick={() => onOpen(app.manifest.appId)}
      >
        {app.manifest.name}
      </button>
      <span className="wb-app-desc">{app.manifest.description}</span>
      {app.preference.favorite && <Star size={12} strokeWidth={1.8} className="wb-app-star" />}
      <button
        type="button"
        className="btn"
        onClick={() => onOpen(app.manifest.appId)}
        data-testid={`wb-open-btn-${app.manifest.appId}`}
      >
        {T.workbench.open}
      </button>
    </li>
  );
}

type CollectionProps = {
  favorites: AppViewDto[];
  rest: AppViewDto[];
  query: string;
  onQuery: (value: string) => void;
  showToday: boolean;
  todayTodos: TodoDto[];
  todayEvents: CalendarEventDto[];
  onOpen: (appId: string) => void;
  onOpenTodo: () => void;
  onOpenCalendar: () => void;
  allOff: boolean;
};

function Collection({
  favorites,
  rest,
  query,
  onQuery,
  showToday,
  todayTodos,
  todayEvents,
  onOpen,
  onOpenTodo,
  onOpenCalendar,
  allOff,
}: CollectionProps) {
  return (
    <>
      <div className="app-toolbar">
        <label className="search-field wb-search">
          <Search size={14} strokeWidth={1.8} />
          <input
            value={query}
            placeholder={W.searchPlaceholder}
            aria-label={W.searchPlaceholder}
            data-testid="workbench-search"
            onChange={(event) => onQuery(event.target.value)}
          />
        </label>
      </div>

      {allOff && <p className="page-empty">{W.noApps}</p>}

      {showToday && (todayTodos.length > 0 || todayEvents.length > 0) && (
        <section className="wb-today" data-testid="workbench-today">
          <h2 className="app-section-title">{W.todayTitle}</h2>
          <div className="wb-today-body">
            {todayTodos.length > 0 && (
              <div className="wb-today-col">
                <button
                  type="button"
                  className="wb-today-head"
                  data-testid="workbench-today-todos"
                  onClick={onOpenTodo}
                >
                  {W.todayTodos} · {todayTodos.length}
                </button>
                <ul className="wb-today-list">
                  {todayTodos.slice(0, MAX_TODAY).map((todo) => (
                    <li key={todo.id}>{todo.title}</li>
                  ))}
                  {todayTodos.length > MAX_TODAY && (
                    <li className="wb-today-more">{W.todayMore(todayTodos.length - MAX_TODAY)}</li>
                  )}
                </ul>
              </div>
            )}
            {todayEvents.length > 0 && (
              <div className="wb-today-col">
                <button
                  type="button"
                  className="wb-today-head"
                  data-testid="workbench-today-events"
                  onClick={onOpenCalendar}
                >
                  {W.todayEvents} · {todayEvents.length}
                </button>
                <ul className="wb-today-list">
                  {todayEvents.slice(0, MAX_TODAY).map((event) => (
                    <li key={event.id}>
                      {event.allDay ? W.allDay : event.startTime} {event.title}
                    </li>
                  ))}
                  {todayEvents.length > MAX_TODAY && (
                    <li className="wb-today-more">{W.todayMore(todayEvents.length - MAX_TODAY)}</li>
                  )}
                </ul>
              </div>
            )}
          </div>
        </section>
      )}

      {favorites.length > 0 && (
        <section className="wb-section">
          <h2 className="app-section-title">{W.sectionFavorites}</h2>
          <ul className="wb-app-list">{favorites.map((app) => appRow(app, onOpen))}</ul>
        </section>
      )}

      <section className="wb-section">
        <h2 className="app-section-title">{W.sectionAll}</h2>
        {rest.length === 0 && favorites.length === 0 ? (
          <p className="page-empty">{query ? W.empty : W.noApps}</p>
        ) : (
          <ul className="wb-app-list">{rest.map((app) => appRow(app, onOpen))}</ul>
        )}
      </section>
    </>
  );
}

type ManageProps = {
  apps: AppViewDto[];
  showToday: boolean;
  onClose: () => void;
  onToggleEnabled: (app: AppViewDto) => void;
  onToggleFavorite: (app: AppViewDto) => void;
  onToggleLink: (app: AppViewDto, link: "ai" | "scheduler") => void;
  onToggleToday: () => void;
  onMove: (appId: string, delta: number) => void;
};

function Manage({
  apps,
  showToday,
  onClose,
  onToggleEnabled,
  onToggleFavorite,
  onToggleLink,
  onToggleToday,
  onMove,
}: ManageProps) {
  const ordered = sortApps(apps);
  return (
    <div className="settings-modal" role="dialog" aria-modal="true" aria-label={W.manage}>
      <div className="settings-modal-backdrop" onClick={onClose} />
      <div className="settings-modal-panel wb-modal-panel">
        <header className="modal-head">
          <span className="modal-title">{W.manage}</span>
          <span className="wb-manage-hint">{W.manageHint}</span>
          <span className="spacer" />
          <button
            type="button"
            className="btn"
            onClick={onClose}
            data-testid="workbench-manage-close"
          >
            {T.action.close}
          </button>
        </header>

        <div className="skill-detail-body">
          <ul className="wb-manage-list">
            {ordered.map((app, index) => (
              <li
                key={app.manifest.appId}
                className="wb-manage-row"
                data-testid={`wb-manage-${app.manifest.appId}`}
              >
                <span className="wb-manage-name">{app.manifest.name}</span>

                <label className="wb-manage-field">
                  <span className="field-label">{W.enable}</span>
                  <button
                    type="button"
                    className={`toggle${app.preference.enabled ? " toggle--on" : ""}`}
                    role="switch"
                    aria-checked={app.preference.enabled}
                    aria-label={`${W.enable} ${app.manifest.name}`}
                    data-testid={`wb-enabled-${app.manifest.appId}`}
                    onClick={() => onToggleEnabled(app)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>

                <label className="wb-manage-field">
                  <span className="field-label">{T.workbench.favoriteAdd}</span>
                  <button
                    type="button"
                    className={`icon-btn icon-btn--sm${app.preference.favorite ? " icon-btn--active" : ""}`}
                    aria-pressed={app.preference.favorite}
                    aria-label={app.preference.favorite ? W.favoriteRemove : W.favoriteAdd}
                    title={app.preference.favorite ? W.favoriteRemove : W.favoriteAdd}
                    data-testid={`wb-favorite-${app.manifest.appId}`}
                    onClick={() => onToggleFavorite(app)}
                  >
                    <Star size={13} strokeWidth={1.8} />
                  </button>
                </label>

                <label className="wb-manage-field">
                  <span className="field-label">
                    {W.moveUp} / {W.moveDown}
                  </span>
                  <span className="wb-manage-pair">
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      aria-label={`${app.manifest.name} ${W.moveUp}`}
                      title={W.moveUp}
                      disabled={index === 0}
                      data-testid={`wb-up-${app.manifest.appId}`}
                      onClick={() => onMove(app.manifest.appId, -1)}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      aria-label={`${app.manifest.name} ${W.moveDown}`}
                      title={W.moveDown}
                      disabled={index === ordered.length - 1}
                      data-testid={`wb-down-${app.manifest.appId}`}
                      onClick={() => onMove(app.manifest.appId, 1)}
                    >
                      ↓
                    </button>
                  </span>
                </label>

                <label className="wb-manage-field">
                  <span className="field-label">{W.aiLink}</span>
                  {app.manifest.capabilities.ai ? (
                    <button
                      type="button"
                      className={`toggle${app.preference.aiEnabled ? " toggle--on" : ""}`}
                      role="switch"
                      aria-checked={app.preference.aiEnabled}
                      aria-label={`${app.manifest.name} ${W.aiLink}`}
                      data-testid={`wb-ai-${app.manifest.appId}`}
                      onClick={() => onToggleLink(app, "ai")}
                    >
                      <span className="toggle-knob" />
                    </button>
                  ) : (
                    <span className="wb-manage-unsupported">{W.linkUnsupported}</span>
                  )}
                </label>

                <label className="wb-manage-field">
                  <span className="field-label">{W.scheduleLink}</span>
                  {app.manifest.capabilities.scheduler ? (
                    <button
                      type="button"
                      className={`toggle${app.preference.scheduleEnabled ? " toggle--on" : ""}`}
                      role="switch"
                      aria-checked={app.preference.scheduleEnabled}
                      aria-label={`${app.manifest.name} ${W.scheduleLink}`}
                      data-testid={`wb-schedule-${app.manifest.appId}`}
                      onClick={() => onToggleLink(app, "scheduler")}
                    >
                      <span className="toggle-knob" />
                    </button>
                  ) : (
                    <span className="wb-manage-unsupported">{W.linkUnsupported}</span>
                  )}
                </label>

                {!app.preference.enabled && (
                  <span className="wb-manage-badge">{W.disabledBadge}</span>
                )}
              </li>
            ))}
          </ul>

          <label className="wb-manage-field">
            <span className="field-label">{W.todayTitle}</span>
            <button
              type="button"
              className={`toggle${showToday ? " toggle--on" : ""}`}
              role="switch"
              aria-checked={showToday}
              aria-label={W.todayTitle}
              data-testid="workbench-summary-toggle"
              onClick={onToggleToday}
            >
              <span className="toggle-knob" />
            </button>
          </label>

          {/* The AI link is declared for 待办 but no authorized AI write channel
              exists yet (see the final report): keep the switch visible and
              off rather than shipping an entry point that cannot act. */}
          {ordered.some((app) => app.manifest.capabilities.ai) && (
            <p className="app-hint">{W.aiGap}</p>
          )}
        </div>

        <footer className="skill-detail-foot">
          <span className="spacer" />
          <button type="button" className="btn btn--primary" onClick={onClose}>
            {T.action.close}
          </button>
        </footer>
      </div>
    </div>
  );
}
