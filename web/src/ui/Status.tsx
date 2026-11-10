import a11y from "./a11y.module.css";
import styles from "./Status.module.css";

/** Tones of a status. The text names the status; the tone only repeats it in color. */
export type Tone = "ok" | "warning" | "danger" | "accent" | "muted";

/** A status in a filled label, as "Active" or "Flagged". */
export function StatusLabel({
  tone,
  children,
}: {
  tone: Tone;
  children: string;
}) {
  return <span className={`${styles.label} ${styles[tone]}`}>{children}</span>;
}

/** A status as a colored dot with text, as "Online". */
export function StatusDot({
  tone,
  children,
}: {
  tone: Tone;
  children: string;
}) {
  return (
    <span className={`${styles.dot} ${styles[tone]}`}>
      <span className={styles.mark} aria-hidden="true" />
      {children}
    </span>
  );
}

/**
 * A count in a badge, as tasks in a queue. `label` follows the number for a screen reader only:
 * `count` 2 with `label` "unread" reads "2 unread".
 */
export function CountBadge({
  tone,
  count,
  label,
}: {
  tone: Tone;
  count: number;
  label?: string;
}) {
  return (
    <span className={`${styles.count} ${styles[tone]}`}>
      {count}
      {label && <span className={a11y.visuallyHidden}> {label}</span>}
    </span>
  );
}
