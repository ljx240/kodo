import { workbenchCode } from "../../api";
import { T } from "../../i18n";

/**
 * zh-CN copy for a failed workbench call. The backend returns a stable code
 * (`revision_conflict`, …) and the GUI owns the wording; anything unrecognised
 * falls back to the generic line rather than showing a raw code.
 */
export function workbenchErrorText(error: unknown): string {
  const code = workbenchCode(error);
  const copy = T.workbench.errors as Record<string, string>;
  return copy[code] ?? T.workbench.errors.unknown;
}
