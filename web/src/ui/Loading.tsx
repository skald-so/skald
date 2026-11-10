import a11y from "./a11y.module.css";
import styles from "./States.module.css";

/** Widths of the placeholder lines, so they read as text of different lengths. */
const widths = ["62%", "44%", "54%", "38%", "48%"];

/**
 * A placeholder while content loads, in place of table rows or anywhere else. A screen reader hears
 * it as busy, with `label`.
 */
export function Loading({
  label = "Loading",
  lines = 3,
}: {
  label?: string;
  lines?: number;
}) {
  return (
    <div className={styles.loading} aria-busy="true">
      <span role="status" className={a11y.visuallyHidden}>
        {label}
      </span>
      {Array.from({ length: lines }, (_, i) => (
        <span
          key={i}
          aria-hidden="true"
          className={styles.line}
          style={{ width: widths[i % widths.length] }}
        />
      ))}
    </div>
  );
}
