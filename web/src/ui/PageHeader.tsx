import type { ReactNode } from "react";
import styles from "./PageHeader.module.css";

/** The top of a cabinet screen: the heading with a subtitle, and actions on the right. */
export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: ReactNode;
  /** Buttons for the whole screen, as "Connect a machine". */
  actions?: ReactNode;
}) {
  return (
    <div className={styles.header}>
      <div className={styles.title}>
        <h1>{title}</h1>
        {subtitle && <p>{subtitle}</p>}
      </div>
      {actions && <div className={styles.actions}>{actions}</div>}
    </div>
  );
}
