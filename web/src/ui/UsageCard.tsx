import { useId } from "react";
import styles from "./UsageCard.module.css";

/** `value` with its thousands separated, the same in every locale: "12,480". */
function amount(value: number): string {
  return value.toLocaleString("en-US");
}

/**
 * The count against its limit or plan allowance, with a bar. An unlimited count has no bar.
 */
export function UsageCard({
  label,
  count,
  limit,
  planAllowance,
}: {
  /** What it counts, as "Devices"; the bar's name starts with it. */
  label: string;
  count: number;
  /** None while no limit applies. */
  limit?: number;
  /** Whether the limit comes from the plan's allowances. */
  planAllowance?: boolean;
}) {
  const id = useId();
  const full = limit !== undefined && count >= limit;
  const limitText = limit === undefined ? "No limit" : `of ${amount(limit)}`;
  const allowance = planAllowance ? " (plan allowance)" : "";
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
        <span className={styles.limit}>{limitText + allowance}</span>
      </span>
      {limit !== undefined && (
        <div
          role="meter"
          aria-labelledby={id}
          aria-valuemin={0}
          aria-valuemax={limit}
          aria-valuenow={count}
          aria-valuetext={`${amount(count)} ${limitText}${allowance}`}
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
