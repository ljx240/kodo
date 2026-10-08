import type { ReactNode } from "react";

/** The settings-row form primitive shared by the Settings page and the MCP
 *  page's inline editor — label column growing left, control column right. */
export function Field({
  icon,
  label,
  hint,
  hintBelow,
  wide,
  block,
  children,
}: {
  icon?: ReactNode;
  label: string;
  hint?: string;
  hintBelow?: boolean;
  wide?: boolean;
  block?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`setting-row${block ? " setting-row--stacked" : ""}`}>
      {icon && <span className="setting-icon">{icon}</span>}
      <div className="setting-text">
        <span className="setting-label">{label}</span>
        {hint && !hintBelow && !block && <span className="setting-hint">{hint}</span>}
      </div>
      <div className={`setting-control${wide ? " setting-control--wide" : ""}`}>
        {children}
        {/* A stacked row's hint lives after the control (below); never both. */}
        {hint && hintBelow && !block && <span className="setting-hint">{hint}</span>}
      </div>
      {hint && block && <span className="setting-hint">{hint}</span>}
    </div>
  );
}

/** The shared switch markup: `Toggle` binds it to a settings key, the MCP
 *  server rows drive it from config state. */
export function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: () => void;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`toggle${checked ? " toggle--on" : ""}`}
      onClick={onChange}
    >
      <span className="toggle-knob" />
    </button>
  );
}
