import { useRef, type ReactNode } from "react";
import styles from "./FilterBar.module.css";

export interface FilterBarProps {
  /** The accessible name of the filters, as "Filter messages". */
  label: string;
  /** The dropdown filters. */
  children: ReactNode;
  /** A filter differs from its default: "Clear" shows. */
  filtered: boolean;
  /** A press on "Clear": the screen resets the filters to their defaults; the sort order stays. */
  onClear: () => void;
  /** The sort order at the end of the bar, a dropdown as "Sort Oldest first". */
  sort?: ReactNode;
}

/**
 * The filter bar of the Messages screen: dropdown filters with "Clear" while one is set, and the
 * sort order at the end. The bar wraps on a narrow screen.
 */
export function FilterBar({
  label,
  children,
  filtered,
  onClear,
  sort,
}: FilterBarProps) {
  const group = useRef<HTMLDivElement>(null);

  return (
    <div className={styles.bar}>
      <div
        ref={group}
        role="group"
        aria-label={label}
        className={styles.filters}
      >
        {children}
        {filtered && (
          <button
            type="button"
            className={styles.clear}
            onClick={() => {
              onClear();
              // "Clear" goes away: the focus moves to the first filter.
              group.current!.querySelector("button")!.focus();
            }}
          >
            Clear
          </button>
        )}
      </div>
      {sort}
    </div>
  );
}
