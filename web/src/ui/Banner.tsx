import type { ReactNode } from "react";
import styles from "./Banner.module.css";
import { Button, type ButtonProps } from "./Button.tsx";

export interface BannerAction extends Pick<
  ButtonProps,
  "onClick" | "disabled" | "busy" | "busyLabel"
> {
  label: string;
}

/**
 * A warning with optional actions, as the agent loop warning. It is an alert: a screen reader
 * announces its text and actions when it appears.
 */
export function Banner({
  icon,
  actions = [],
  children,
}: {
  icon?: ReactNode;
  actions?: BannerAction[];
  children: ReactNode;
}) {
  return (
    <div role="alert" className={styles.banner}>
      <p className={styles.message}>
        {icon && (
          <span className={styles.icon} aria-hidden="true">
            {icon}
          </span>
        )}
        <span>{children}</span>
      </p>
      {actions.length > 0 && (
        <div className={styles.actions}>
          {actions.map(({ label, ...action }) => (
            <Button
              key={label}
              {...action}
              size="small"
              className={styles.action}
            >
              {label}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
