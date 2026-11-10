import type { ButtonHTMLAttributes } from "react";
import a11y from "./a11y.module.css";
import styles from "./Button.module.css";

/**
 * - `primary`: the main action, as "Connect a machine" or "Approve".
 * - `secondary`: any other action, as "Reject" or "Manage".
 * - `danger`: an action that revokes or deletes, as "Revoke" or "Remove".
 * - `danger-primary`: the main action of a dangerous item, as "Ban account".
 */
export type ButtonKind = "primary" | "secondary" | "danger" | "danger-primary";

/** `large`: the page action next to the heading; `medium`: cards and dialogs; `small`: table rows. */
export type ButtonSize = "large" | "medium" | "small";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  kind?: ButtonKind;
  size?: ButtonSize;
  /**
   * The action is running: the button keeps its focus, ignores presses and shows a spinner.
   * Passing `busy`, even `false`, adds the live region that announces `busyLabel` when it turns on.
   */
  busy?: boolean;
  busyLabel?: string;
}

/** A button of the design; `type` is `button` unless set. */
export function Button({
  kind = "secondary",
  size = "medium",
  busy,
  busyLabel = "Busy",
  type = "button",
  className,
  onClick,
  children,
  ...props
}: ButtonProps) {
  return (
    <>
      <button
        {...props}
        type={type}
        className={[styles.button, styles[kind], styles[size], className]
          .filter(Boolean)
          .join(" ")}
        aria-disabled={busy || props["aria-disabled"]}
        aria-busy={busy || undefined}
        onClick={(event) => {
          // A busy submit button must not submit its form either.
          if (busy) event.preventDefault();
          else onClick?.(event);
        }}
      >
        {busy && <span className={styles.spinner} aria-hidden="true" />}
        {children}
      </button>
      {busy !== undefined && (
        <span role="status" className={a11y.visuallyHidden}>
          {busy ? busyLabel : ""}
        </span>
      )}
    </>
  );
}
