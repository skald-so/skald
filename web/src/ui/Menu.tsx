import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import styles from "./Menu.module.css";

export interface MenuAction {
  label: string;
  onSelect: () => void;
}

export interface MenuProps {
  /** The name of the button and of the menu, as "Account". */
  label: string;
  /** The button face, an icon; the name is for a screen reader. */
  icon: ReactNode;
  actions: readonly MenuAction[];
  /** Opens the menu on the first render without taking the focus, as on the kit page. */
  defaultOpen?: boolean;
}

/**
 * A button with a menu of actions above it, as the account menu at the foot of the side menu.
 *
 * Enter, Space or Down on the button opens the menu on the first action, Up on the last; arrows,
 * Home and End move, Enter or Space chooses. Escape, a click outside or Tab closes it without a
 * choice. The focus returns to the button.
 */
export function Menu({ label, icon, actions, defaultOpen = false }: MenuProps) {
  const id = useId();
  const frame = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(defaultOpen);
  /** The action that takes the focus once the menu opens; none when it is open from the first render. */
  const focusOn = useRef<number | null>(null);

  function show(index: number) {
    focusOn.current = index;
    setOpen(true);
  }

  /** Closes the menu; `refocus` returns the focus to the button. */
  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) button.current!.focus();
  }

  function choose(action: MenuAction) {
    close(true);
    action.onSelect();
  }

  const items = () => [
    ...menu.current!.querySelectorAll<HTMLElement>("[role=menuitem]"),
  ];

  useLayoutEffect(() => {
    if (!open || focusOn.current === null) return;
    items()[focusOn.current]?.focus();
    focusOn.current = null;
  }, [open]);

  // A press outside closes the menu without a choice.
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!frame.current!.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  function onMenuKey(event: KeyboardEvent) {
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLElement);
    const last = all.length - 1;
    const moves: Record<string, number> = {
      ArrowDown: at < last ? at + 1 : 0,
      ArrowUp: at > 0 ? at - 1 : last,
      Home: 0,
      End: last,
    };
    if (event.key in moves) {
      event.preventDefault();
      all[moves[event.key]!]!.focus();
    } else if (event.key === "Escape") {
      // Escape closes only the menu, not the side menu around it.
      event.preventDefault();
      event.stopPropagation();
      close(true);
    } else if (event.key === "Tab") {
      // Tab goes on from the button, as if the menu were not there.
      close(true);
    }
  }

  return (
    <div
      ref={frame}
      className={styles.frame}
      onBlur={(event) => {
        if (open && !frame.current!.contains(event.relatedTarget)) close(false);
      }}
    >
      <button
        ref={button}
        type="button"
        className={styles.button}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? `${id}-menu` : undefined}
        onClick={() => (open ? close(false) : show(0))}
        onKeyDown={(event) => {
          const opens: Record<string, number> = {
            ArrowDown: 0,
            ArrowUp: actions.length - 1,
          };
          const index = opens[event.key];
          if (index === undefined) return;
          event.preventDefault();
          if (open) items()[index]!.focus();
          else show(index);
        }}
      >
        {icon}
      </button>
      {open && (
        <div
          ref={menu}
          id={`${id}-menu`}
          role="menu"
          aria-label={label}
          className={styles.menu}
          onKeyDown={onMenuKey}
        >
          {actions.map((action) => (
            <button
              key={action.label}
              type="button"
              role="menuitem"
              tabIndex={-1}
              className={styles.item}
              onClick={() => choose(action)}
            >
              {action.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
