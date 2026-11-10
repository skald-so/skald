import type { ReactNode } from "react";
import styles from "./States.module.css";

/** A short message where a list has nothing to show, with an optional action, as a button. */
export function EmptyState({
  action,
  children,
}: {
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className={styles.empty}>
      <p>{children}</p>
      {action}
    </div>
  );
}
