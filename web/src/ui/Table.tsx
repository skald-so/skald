import {
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
  type RefObject,
} from "react";
import a11y from "./a11y.module.css";
import { Loading } from "./Loading.tsx";
import { nextSort, type Sort } from "./sort.ts";
import type { Tone } from "./Status.tsx";
import styles from "./Table.module.css";

export interface Column<Row> {
  /** Names the column in `Sort`. */
  key: string;
  /** The header text. */
  label: string;
  /** A click on the header reports the column to `onSort`. */
  sortable?: boolean;
  /** A grid track, the same in every row: `minmax(0, 2fr)` or `120px`. Default `minmax(0, 1fr)`. */
  width?: string;
  /** Muted text, as the kind of a device. */
  muted?: boolean;
  /** A value that needs attention, bold in the tone, as the dead letters of a queue. */
  emphasis?: (row: Row) => Tone | undefined;
  /** The actions column: the header is for a screen reader only and the buttons fill the cell. */
  actions?: boolean;
  cell: (row: Row) => ReactNode;
}

export interface TableProps<Row> {
  /** The accessible name, as "Devices". */
  label: string;
  columns: Column<Row>[];
  rows: readonly Row[];
  rowKey: (row: Row) => string;
  /** The current order, set by the screen; the screen sorts the rows itself or asks the API. */
  sort?: Sort;
  /** A click on a sortable header: its column, ascending, or the other direction if it is active. */
  onSort?: (sort: Sort) => void;
  /** A row that needs attention, as a queue with dead letters. */
  highlight?: (row: Row) => boolean;
  /** Shows a placeholder in place of the rows. */
  loading?: boolean;
  /** What a screen reader hears while loading, as "Loading devices". */
  loadingLabel?: string;
  /** Shown in place of the rows when there are none, usually an `EmptyState`. */
  empty: ReactNode;
  /** The width below which the table scrolls inside its frame. */
  minWidth?: number;
}

/**
 * A table of the design screens: sortable headers with an arrow on the active column, rows of the
 * same grid tracks, an empty and a loading state in place of the rows.
 */
export function Table<Row>({
  label,
  columns,
  rows,
  rowKey,
  sort,
  onSort,
  highlight,
  loading,
  loadingLabel = "Loading",
  empty,
  minWidth,
}: TableProps<Row>) {
  const frame = useRef<HTMLDivElement>(null);
  const { scrolls, width } = useFrameWidth(frame);
  const active = columns.find(
    (column) => column.sortable && column.key === sort?.column,
  );
  const tracks = columns
    .map((column) => column.width ?? "minmax(0, 1fr)")
    .join(" ");
  const placeholder = loading ? (
    <Loading label={loadingLabel} />
  ) : rows.length === 0 ? (
    empty
  ) : null;

  return (
    // A frame that scrolls takes the focus, so the keyboard scrolls it too.
    <div
      ref={frame}
      className={styles.frame}
      tabIndex={scrolls ? 0 : undefined}
      role={scrolls ? "region" : undefined}
      aria-label={scrolls ? label : undefined}
    >
      <table
        aria-label={label}
        aria-busy={loading || undefined}
        className={styles.table}
        style={
          {
            "--tracks": tracks,
            "--frame-width": width ? `${width}px` : undefined,
            minWidth: minWidth && `${minWidth}px`,
          } as CSSProperties
        }
      >
        <thead>
          <tr>
            {columns.map((column) => (
              <th
                key={column.key}
                scope="col"
                aria-sort={column === active ? sort!.direction : undefined}
              >
                {column.actions ? (
                  <span className={a11y.visuallyHidden}>{column.label}</span>
                ) : column.sortable ? (
                  <button
                    type="button"
                    className={styles.sort}
                    onClick={() => onSort?.(nextSort(sort, column.key))}
                  >
                    {column.label}
                    <span
                      aria-hidden="true"
                      className={column === active ? undefined : styles.idle}
                    >
                      {column !== active
                        ? "↕"
                        : sort!.direction === "ascending"
                          ? "↑"
                          : "↓"}
                    </span>
                  </button>
                ) : (
                  column.label
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {placeholder !== null ? (
            <tr>
              <td colSpan={columns.length} className={styles.placeholder}>
                {placeholder}
              </td>
            </tr>
          ) : (
            rows.map((row) => (
              <tr
                key={rowKey(row)}
                className={highlight?.(row) ? styles.highlight : undefined}
              >
                {columns.map((column) => {
                  const tone = column.emphasis?.(row);
                  return (
                    <td
                      key={column.key}
                      className={[
                        column.muted && styles.muted,
                        column.actions && styles.actions,
                        tone && styles.emphasis,
                        tone && styles[tone],
                      ]
                        .filter(Boolean)
                        .join(" ")}
                    >
                      {column.cell(row)}
                    </td>
                  );
                })}
              </tr>
            ))
          )}
        </tbody>
      </table>
      <span role="status" className={a11y.visuallyHidden}>
        {active && `Sorted by ${active.label}, ${sort!.direction}`}
      </span>
    </div>
  );
}

/** Two lines in a cell: a value and a detail under it, as a device and "this browser". */
export function TwoLine({
  children,
  detail,
  detailTone = "muted",
  mono,
}: {
  children: ReactNode;
  detail?: ReactNode;
  detailTone?: Tone;
  /** The value is a key or an ID. */
  mono?: boolean;
}) {
  return (
    <span className={styles.lines}>
      <span className={mono ? styles.code : styles.name}>{children}</span>
      {detail && (
        <span className={`${styles.detail} ${styles[detailTone]}`}>
          {detail}
        </span>
      )}
    </span>
  );
}

/** A name that is an ID, as a queue. */
export function Mono({ children }: { children: ReactNode }) {
  return <span className={styles.mono}>{children}</span>;
}

/**
 * The visible width of the element and whether it scrolls sideways, kept up to date as it or its
 * content resizes. Zero before layout and where there is none, as in jsdom.
 */
function useFrameWidth(ref: RefObject<HTMLElement | null>): {
  scrolls: boolean;
  width: number;
} {
  const [frame, setFrame] = useState({ scrolls: false, width: 0 });

  useLayoutEffect(() => {
    const element = ref.current!;
    const check = () => {
      const scrolls = element.scrollWidth > element.clientWidth;
      const width = element.clientWidth;
      setFrame((frame) =>
        frame.scrolls === scrolls && frame.width === width
          ? frame
          : { scrolls, width },
      );
    };
    check();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(check);
    observer.observe(element);
    observer.observe(element.firstElementChild!);
    return () => observer.disconnect();
  }, [ref]);

  return frame;
}
