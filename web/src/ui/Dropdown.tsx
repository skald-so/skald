import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { CheckIcon, ChevronIcon } from "./icons.tsx";
import styles from "./Dropdown.module.css";

export interface DropdownOption<Value extends string> {
  value: Value;
  /** The text in the list and, once chosen, on the button, as "anyone". */
  label: string;
}

export interface DropdownProps<Value extends string> {
  /** The filter name on the button and the name of the list, as "From" or "Sort". */
  label: string;
  options: readonly DropdownOption<Value>[];
  /** The current value, set by the screen. */
  value: Value;
  /** The value of a cleared filter; any other value is highlighted. Without it nothing is. */
  defaultValue?: Value;
  /** A choice of another value. Choosing the current one reports nothing. */
  onChange: (value: Value) => void;
  /** Opens the list on the first render without taking the focus, as on the kit page. */
  defaultOpen?: boolean;
  /**
   * The name above the value on a button as wide as its place, as the space switcher of the side
   * menu; otherwise the name and the value in one line, as a filter.
   */
  stacked?: boolean;
}

/** Space kept between the list and the window edges, in pixels. */
const margin = 8;

/**
 * A dropdown filter of the design screens, as "From anyone" or "Sort Oldest first": a button with
 * the filter name and the current value that opens a list of options.
 *
 * Enter, Space or an arrow key on the button opens the list; arrows, Home and End move, Enter or
 * Space chooses; Escape, a click outside or leaving it with Tab closes it without a change. The
 * focus returns to the button. The list stays inside the window.
 */
export function Dropdown<Value extends string>({
  label,
  options,
  value,
  defaultValue,
  onChange,
  defaultOpen = false,
  stacked = false,
}: DropdownProps<Value>) {
  const id = useId();
  const frame = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const current = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );
  const [open, setOpen] = useState(defaultOpen);
  const [active, setActive] = useState(current);
  /** The list takes the focus once it opens: not when it is open from the first render. */
  const focusList = useRef(false);

  function show() {
    setActive(current);
    focusList.current = true;
    setOpen(true);
  }

  /** Closes the list; `refocus` returns the focus to the button. */
  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) button.current!.focus();
  }

  function choose(index: number) {
    const option = options[index]!;
    close(true);
    if (option.value !== value) onChange(option.value);
  }

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => fitInWindow(frame.current!, list.current!);
    place();
    // Focused once in place: in the window, so the focus does not scroll the page.
    if (focusList.current) {
      focusList.current = false;
      list.current!.focus();
    }
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open]);

  // A press outside closes the list without a change.
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!frame.current!.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  // A list taller than its room scrolls to the active option; the page itself does not scroll.
  useLayoutEffect(() => {
    if (!open) return;
    const box = list.current!;
    const option = box.children[active] as HTMLElement;
    if (option.offsetTop < box.scrollTop) box.scrollTop = option.offsetTop;
    const bottom = option.offsetTop + option.offsetHeight - box.clientHeight;
    if (bottom > box.scrollTop) box.scrollTop = bottom;
  }, [open, active]);

  function onListKey(event: KeyboardEvent) {
    const moves: Record<string, number> = {
      ArrowDown: Math.min(active + 1, options.length - 1),
      ArrowUp: Math.max(active - 1, 0),
      Home: 0,
      End: options.length - 1,
    };
    if (event.key in moves) {
      event.preventDefault();
      setActive(moves[event.key]!);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      choose(active);
    } else if (event.key === "Escape") {
      // Escape closes only the list, not a dialog around it.
      event.preventDefault();
      event.stopPropagation();
      close(true);
    } else if (event.key === "Tab") {
      // Tab goes on from the button, as if the list were not there.
      close(true);
    }
  }

  const optionId = (index: number) => `${id}-option-${index}`;

  return (
    <div
      ref={frame}
      className={[styles.dropdown, stacked && styles.stacked]
        .filter(Boolean)
        .join(" ")}
      onBlur={(event) => {
        if (open && !frame.current!.contains(event.relatedTarget)) close(false);
      }}
    >
      <button
        ref={button}
        type="button"
        className={styles.button}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-list` : undefined}
        data-set={
          (defaultValue !== undefined && value !== defaultValue) || undefined
        }
        onClick={() => (open ? close(false) : show())}
        onKeyDown={(event) => {
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          event.preventDefault();
          if (!open) show();
          else list.current!.focus();
        }}
      >
        {/* A screen reader hears "From anyone" or "Space team-a", as shown. */}
        <span className={styles.label}>{label}</span>{" "}
        <span className={styles.value}>{options[current]?.label}</span>
        <ChevronIcon size={stacked ? 16 : 14} />
      </button>
      {open && (
        <ul
          ref={list}
          id={`${id}-list`}
          role="listbox"
          aria-label={label}
          aria-activedescendant={optionId(active)}
          tabIndex={-1}
          className={styles.list}
          onKeyDown={onListKey}
        >
          {options.map((option, index) => (
            <li
              key={option.value}
              id={optionId(index)}
              role="option"
              aria-selected={index === current}
              className={[styles.option, index === active && styles.active]
                .filter(Boolean)
                .join(" ")}
              onPointerMove={() => setActive(index)}
              onClick={() => choose(index)}
            >
              <CheckIcon />
              {option.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Moves the list of `frame` so that it stays inside the window: sideways, and above the button
 * when it does not fit below and there is more room above. A list taller than its room scrolls.
 * A button out of sight, as one open from the first render, keeps the list below it.
 */
function fitInWindow(frame: HTMLElement, list: HTMLElement) {
  const { clientWidth, clientHeight } = document.documentElement;
  Object.assign(list.style, {
    left: "0px",
    top: "",
    bottom: "",
    maxWidth: `${clientWidth - 2 * margin}px`,
    maxHeight: "",
  });
  const box = frame.getBoundingClientRect();
  const { width, height } = list.getBoundingClientRect();
  const left = Math.max(
    margin,
    Math.min(box.left, clientWidth - margin - width),
  );
  list.style.left = `${left - box.left}px`;

  if (box.top < 0 || box.bottom > clientHeight) return;
  // The gap between the button and the list, from the style.
  const gap = list.offsetTop - box.height;
  const below = clientHeight - margin - box.bottom - gap;
  const above = box.top - margin - gap;
  const up = height > below && above > below;
  if (up) {
    list.style.top = "auto";
    list.style.bottom = `${box.height + gap}px`;
  }
  list.style.maxHeight = `${up ? above : below}px`;
}
