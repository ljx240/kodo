import { useEffect, useState } from "react";
import { isDesktop } from "./api";

/**
 * The deterministic demo routes from docs/design/DEMO_DATA.md.
 * Plain `location` reads keep this dependency-free; the app never needs a
 * router for four static screens.
 */
export type RouteName =
  | "conversation"
  | "plugins"
  | "trace"
  | "archive"
  | "settings"
  | "schedule";

/** The active tab of the 插件 page — `?type=`, default `skills`. */
export type PluginType = "skills" | "mcp" | "agents";

export type Route = {
  name: RouteName;
  /** Tab of the 插件 page; meaningful only when `name === "plugins"`. */
  pluginType: PluginType;
  inspectorOpen: boolean;
  /**
   * True under `/ui-demo`. Those routes are the deterministic screenshot states
   * from docs/design/DEMO_DATA.md, so they read the fixture even in the desktop
   * shell; the shell navigates the plain routes, which read the real workspace.
   */
  demo: boolean;
};

const NAMES: Record<string, RouteName> = {
  conversation: "conversation",
  plugins: "plugins",
  trace: "trace",
  archive: "archive",
  settings: "settings",
  schedule: "schedule",
};

/**
 * The `/ui-demo` family exists for visual regression: those addresses have to
 * keep rendering the fixture wherever they are opened, which is what makes the
 * screenshots reproducible. Everything else is the live application.
 */
const DEMO_PREFIX = "/ui-demo";

function parse(): Route {
  const params = new URLSearchParams(window.location.search);
  const path = window.location.pathname;
  const demo = path.startsWith(DEMO_PREFIX);
  const key = (demo ? path.slice(DEMO_PREFIX.length) : path).replace(/^\//, "");

  // Inspector default (2026-10-13):
  //
  // - **Production path** (`window.__KODO_INSPECTOR_AUTOPEN__ !== true`):
  //   the right column is hidden until the user opens it from the rail.
  //   Default closed on every viewport; the rail stays the only entry
  //   point and `?inspector=open` is the explicit override.
  //
  // - **E2E / visual-regression path** (the test harness sets
  //   `__KODO_INSPECTOR_AUTOPEN__ = true` in its `addInitScript`,
  //   see `apps/desktop/tests/visual/shell.ts`): the wide-viewport
  //   keeps the panel open by default so the suite's existing
  //   assertions on Inspector geometry, the file / browse panels, and
  //   the LLM section do not need to chase the production default. The
  //   E2E path is opt-in by the harness; it never leaks into the
  //   desktop shell or the user's browser.
  //
  // Explicit `?inspector=open|closed` always wins so a sticky user
  // choice (or a Playwright assertion that needs the panel) is
  // preserved across navigation and reload.
  const raw = params.get("inspector");
  const autopen =
    typeof window !== "undefined" &&
    (window as unknown as { __KODO_INSPECTOR_AUTOPEN__?: boolean })
      .__KODO_INSPECTOR_AUTOPEN__ === true;
  let inspectorOpen: boolean;
  if (raw === "closed") {
    inspectorOpen = false;
  } else if (raw === "open") {
    inspectorOpen = true;
  } else if (autopen) {
    const narrow =
      typeof window !== "undefined" &&
      window.matchMedia("(max-width: 1199px)").matches;
    inspectorOpen = !narrow;
  } else {
    inspectorOpen = false;
  }

  // The 插件 page owns the old /skills and /mcp destinations as permanent
  // aliases (with and without the demo prefix), so existing links keep
  // landing on the right tab. Unknown `?type=` falls back to skills.
  if (key === "plugins" || key === "skills" || key === "mcp") {
    const rawType = key === "plugins" ? params.get("type") ?? "skills" : key;
    const pluginType: PluginType = rawType === "mcp" || rawType === "agents" ? rawType : "skills";
    return { name: "plugins", pluginType, inspectorOpen, demo };
  }

  return {
    // An unknown path is the live conversation, which is also where `/` lands.
    name: NAMES[key] ?? "conversation",
    pluginType: "skills",
    inspectorOpen,
    demo,
  };
}

export function navigate(to: string): void {
  window.history.pushState(null, "", to);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

export function useRoute(): Route {
  const [route, setRoute] = useState(parse);

  useEffect(() => {
    const sync = () => setRoute(parse());
    window.addEventListener("popstate", sync);
    return () => window.removeEventListener("popstate", sync);
  }, []);

  return route;
}

/**
 * Route for a sidebar entry, preserving the current Inspector state.
 *
 * The shell links to the live routes so that every screen it can reach shows
 * real data; the browser keeps the demo family, which is what the visual
 * regression suite loads.
 */
export function hrefTo(name: RouteName, inspectorOpen: boolean, type?: PluginType): string {
  const path = `${isDesktop() ? "" : DEMO_PREFIX}/${name}`;
  const params = new URLSearchParams();
  // The default tab emits no param, so wide-viewport screenshots stay
  // query-free; only a non-default tab asks for `?type=`.
  if (type && type !== "skills") params.set("type", type);
  // Inspector default flipped to "closed on every viewport" in 2026-10-13
  // for the production path (see `parse()` above for the E2E / autopen
  // carve-out). The href only needs to spell out the state when the
  // caller is asking for the non-default open. `?inspector=closed` is
  // omitted because it now matches the default.
  if (inspectorOpen) params.set("inspector", "open");
  const query = params.toString();
  return query ? `${path}?${query}` : path;
}
