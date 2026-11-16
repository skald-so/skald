import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Button } from "./Button.tsx";
import styles from "./ConfirmDialog.module.css";

export interface ConfirmDialogProps {
  /** The question, as "Sign out everywhere else?". */
  title: string;
  /** What the action does. */
  children: ReactNode;
  /** The name of the button that confirms, as "Sign out everywhere else". */
  confirm: string;
  onConfirm: () => void;
  /** Cancel, Escape or a click beside the dialog. */
  onCancel: () => void;
  /** The action is running: the confirm button shows a spinner and ignores presses. */
  busy?: boolean;
}

/**
 * A modal question before a dangerous action, over the page: Cancel and the action. It takes the
 * focus on Cancel and keeps Tab inside; Escape or a click beside it cancels. The focus returns to
 * where it was when it closes.
 */
export function ConfirmDialog({
  title,
  children,
  confirm,
  onConfirm,
  onCancel,
  busy,
}: ConfirmDialogProps) {
  const id = useId();
  const dialog = useRef<HTMLDivElement>(null);

  const buttons = () => [
    ...dialog.current!.querySelectorAll<HTMLElement>("button"),
  ];

  useLayoutEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    buttons()[0]!.focus();
    return () => before?.focus();
  }, []);

  // Escape anywhere cancels, also with the focus outside, as after a click on the page.
  useEffect(() => {
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [onCancel]);

  function onKey(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const all = buttons();
    const at = all.indexOf(document.activeElement as HTMLElement);
    const next = event.shiftKey
      ? at <= 0
        ? all.length - 1
        : at - 1
      : at === all.length - 1
        ? 0
        : at + 1;
    event.preventDefault();
    all[next]!.focus();
  }

  return (
    <div
      className={styles.backdrop}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div
        ref={dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        aria-describedby={`${id}-text`}
        className={styles.dialog}
        onKeyDown={onKey}
      >
        <h2 id={`${id}-title`}>{title}</h2>
        <div id={`${id}-text`} className={styles.text}>
          {children}
        </div>
        <div className={styles.actions}>
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            kind="danger-primary"
            busy={busy}
            busyLabel={confirm}
            onClick={onConfirm}
          >
            {confirm}
          </Button>
        </div>
      </div>
    </div>
  );
}
