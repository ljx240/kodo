import { useState, type ReactNode } from "react";

type MenuProps = {
  /** Draws the trigger. The caller keeps its own class names; the menu only owns
   *  the open state, so a chip and an icon button can use the same panel. */
  trigger: (props: { open: boolean; toggle: () => void }) => ReactNode;
  /** Right-aligns the panel, for a trigger near the edge of its container. */
  align?: "right";
  children: (close: () => void) => ReactNode;
};

/**
 * A click-to-open panel anchored to its trigger.
 *
 * The backdrop is what closes it on the next click anywhere, which is why the
 * panel needs no outside-click listener of its own.
 */
export function Menu({ trigger, align, children }: MenuProps) {
  const [open, setOpen] = useState(false);

  return (
    <div className="menu-anchor">
      {trigger({ open, toggle: () => setOpen((value) => !value) })}

      {open && (
        <>
          <div className="menu-backdrop" onClick={() => setOpen(false)} />
          <div className={`menu${align === "right" ? " menu--right" : ""}`} role="menu">
            {children(() => setOpen(false))}
          </div>
        </>
      )}
    </div>
  );
}

export function MenuItem({
  icon,
  label,
  hint,
  danger,
  onSelect,
}: {
  icon: ReactNode;
  label: string;
  hint?: string;
  danger?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className={`menu-item${danger ? " menu-item--danger" : ""}`}
      onClick={onSelect}
    >
      {icon}
      <span>{label}</span>
      {hint && <span className="menu-item-hint">{hint}</span>}
    </button>
  );
}
