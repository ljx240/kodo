import { useEffect, useState } from "react";
import { isDesktop } from "./api";

/**
 * The deterministic demo routes from docs/design/DEMO_DATA.md.
 * Plain `location` reads keep this dependency-free; the app never needs a
 * router for four static screens.
 */
export type RouteName = "conversation" | "trace" | "archive" | "settings";

export type Route = {
  name: RouteName;
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
  trace: "trace",
  archive: "archive",
  settings: "settings",
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

  return {
    // An unknown path is the live conversation, which is also where `/` lands.
    name: NAMES[key] ?? "conversation",
    inspectorOpen: params.get("inspector") !== "closed",
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
export function hrefTo(name: RouteName, inspectorOpen: boolean): string {
  const path = `${isDesktop() ? "" : DEMO_PREFIX}/${name}`;
  return inspectorOpen ? path : `${path}?inspector=closed`;
}
