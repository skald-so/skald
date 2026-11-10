import { useRef, type CSSProperties } from "react";
import { CrossIcon } from "./icons.tsx";
import styles from "./Search.module.css";

export interface SearchProps {
  /** The accessible name, as "Search accounts". */
  label: string;
  /** The hint in the empty field, as "Search by name, login or email". */
  placeholder?: string;
  /** The query, set by the screen. */
  value: string;
  /** Every change of the query as the user types; an empty one when it is cleared. */
  onChange: (query: string) => void;
  /** The width in pixels on a wide screen; the field never gets wider than its container. */
  width?: number;
}

/**
 * A search field of the design screens. While it holds a query, a button and Escape clear it; the
 * focus stays in the field.
 */
export function Search({
  label,
  placeholder,
  value,
  onChange,
  width = 280,
}: SearchProps) {
  const input = useRef<HTMLInputElement>(null);

  return (
    <div
      className={styles.search}
      style={{ "--search-width": `${width}px` } as CSSProperties}
    >
      <input
        ref={input}
        type="search"
        aria-label={label}
        placeholder={placeholder}
        value={value}
        className={styles.field}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          // An empty field lets Escape through, as to close a dialog around it.
          if (event.key !== "Escape" || value === "") return;
          event.preventDefault();
          event.stopPropagation();
          onChange("");
        }}
      />
      {value !== "" && (
        <button
          type="button"
          aria-label="Clear search"
          className={styles.clear}
          onClick={() => {
            onChange("");
            input.current!.focus();
          }}
        >
          <CrossIcon />
        </button>
      )}
    </div>
  );
}
