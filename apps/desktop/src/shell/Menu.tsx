import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

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
 * Keyboard contract: Escape / ArrowUp / ArrowDown / Home / End move focus
 * inside the panel; closing returns focus to the trigger unless something else
 * already claimed it (an autoFocus input, for example).
 *
 * The backdrop is what closes it on the next click anywhere, which is why the
 * panel needs no outside-click listener of its own.
 */
export function Menu({ trigger, align, children }: MenuProps) {
  const [open, setOpen] = useState(false);
  const anchorRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const menuId = useId();

  const triggerEl = () =>
    anchorRef.current?.querySelector<HTMLElement>("button, [role='button'], a") ?? null;

  const close = (returnFocus: boolean) => {
    const focusWasInMenu = menuRef.current?.contains(document.activeElement) ?? false;
    setOpen(false);
    if (!returnFocus) return;
    requestAnimationFrame(() => {
      const active = document.activeElement;
      const lost =
        !active || active === document.body || active === document.documentElement;
      if (focusWasInMenu && lost) triggerEl()?.focus();
      else if (lost) triggerEl()?.focus();
    });
  };

  useEffect(() => {
    const btn = triggerEl();
    if (!btn) return;
    btn.setAttribute("aria-haspopup", "menu");
    btn.setAttribute("aria-expanded", String(open));
    if (open) btn.setAttribute("aria-controls", menuId);
    else btn.removeAttribute("aria-controls");
  }, [open, menuId]);

  useEffect(() => {
    if (!open) return;
    const first = menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]');
    first?.focus();
  }, [open]);

  const onAnchorKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!open) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close(true);
    }
  };

  const onMenuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!open) return;
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [],
    );
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLElement);
    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        items[(current + 1) % items.length]?.focus();
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        items[(current - 1 + items.length) % items.length]?.focus();
        break;
      }
      case "Home": {
        event.preventDefault();
        items[0]?.focus();
        break;
      }
      case "End": {
        event.preventDefault();
        items[items.length - 1]?.focus();
        break;
      }
      case "Escape": {
        event.preventDefault();
        event.stopPropagation();
        close(true);
        break;
      }
      case "Tab": {
        // Leave the panel; focus returns to the trigger so Tab continues from
        // a predictable place rather than vanishing into body.
        event.preventDefault();
        close(false);
        triggerEl()?.focus();
        break;
      }
      default:
        break;
    }
  };

  return (
    <div className="menu-anchor" ref={anchorRef} onKeyDown={onAnchorKeyDown}>
      {trigger({
        open,
        toggle: () => {
          if (open) close(true);
          else setOpen(true);
        },
      })}

      {open && (
        <>
          <div className="menu-backdrop" onClick={() => close(false)} />
          <div
            ref={menuRef}
            id={menuId}
            className={`menu${align === "right" ? " menu--right" : ""}`}
            role="menu"
            onKeyDown={onMenuKeyDown}
          >
            {children(() => close(true))}
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
