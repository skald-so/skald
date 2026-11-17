import { useId } from "react";
import styles from "./UsageCard.module.css";

/** `value` with its thousands separated, the same in every locale: "12,480". */
function amount(value: number): string {
  return value.toLocaleString("en-US");
}

/**
 * A count against its limit, a card of 11-usage.png: what it counts, the count "of" the limit and
 * a bar that fills up to the limit. Without a limit, the count and "No limit", with no bar.
 */
export function UsageCard({
  label,
  count,
  limit,
}: {
  /** What it counts, as "Devices"; the bar's name starts with it. */
  label: string;
  count: number;
  /** None while no limit applies. */
  limit?: number;
}) {
  const id = useId();
  const full = limit !== undefined && count >= limit;
  // A count above zero shows on the bar, however small its share.
  const share =
    limit === undefined
      ? 0
      : Math.max(Math.min(count / limit, 1), count && 0.01);
  return (
    <div className={styles.card}>
      <span id={id} className={styles.label}>
        {label}
      </span>
      <span className={styles.value}>
        {amount(count)}{" "}
        <span className={styles.limit}>
          {limit === undefined ? "No limit" : `of ${amount(limit)}`}
        </span>
      </span>
      {limit !== undefined && (
        <div
          role="meter"
          aria-labelledby={id}
          aria-valuemin={0}
          aria-valuemax={limit}
          aria-valuenow={count}
          aria-valuetext={`${amount(count)} of ${amount(limit)}`}
          className={styles.bar}
        >
          <div
            className={full ? `${styles.fill} ${styles.full}` : styles.fill}
            style={{ width: `${share * 100}%` }}
          />
        </div>
      )}
    </div>
  );
}
