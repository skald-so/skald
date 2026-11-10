import { useEffect, useState } from "react";
import { Button, type ButtonKind, type ButtonSize } from "../ui/Button.tsx";
import styles from "./KitPage.module.css";

const kinds: { kind: ButtonKind; name: string; use: string; label: string }[] =
  [
    {
      kind: "primary",
      name: "Primary",
      use: "the main action",
      label: "Approve",
    },
    {
      kind: "secondary",
      name: "Secondary",
      use: "other actions",
      label: "Reject",
    },
    {
      kind: "danger",
      name: "Danger",
      use: "revokes or deletes",
      label: "Revoke",
    },
    {
      kind: "danger-primary",
      name: "Danger primary",
      use: "the main action of a dangerous item",
      label: "Ban account",
    },
  ];

const states = [
  { name: "Default" },
  { name: "Disabled", disabled: true },
  { name: "Busy", busy: true },
];

const sizes: {
  size: ButtonSize;
  kind: ButtonKind;
  name: string;
  label: string;
}[] = [
  { size: "large", kind: "primary", name: "Large", label: "Connect a machine" },
  { size: "medium", kind: "primary", name: "Medium", label: "Approve" },
  { size: "small", kind: "secondary", name: "Small", label: "Manage" },
];

/** How long a sample action stays busy after a press. */
export const busyMs = 2000;

export function KitButtons() {
  return (
    <section aria-labelledby="kit-buttons" className={styles.section}>
      <h2 id="kit-buttons">Buttons</h2>
      <ul className={styles.cards}>
        {kinds.map((item) => (
          <li key={item.kind} className={styles.card}>
            <span className={styles.name}>
              {item.name} <span className={styles.use}>· {item.use}</span>
            </span>
            <div className={styles.samples}>
              {states.map((state) => (
                <div key={state.name} className={styles.sample}>
                  <Button
                    kind={item.kind}
                    disabled={state.disabled}
                    busy={state.busy}
                  >
                    {item.label}
                  </Button>
                  <span className={styles.caption}>{state.name}</span>
                </div>
              ))}
            </div>
          </li>
        ))}
        <li className={styles.card}>
          <span className={styles.name}>
            Sizes{" "}
            <span className={styles.use}>
              · page action, cards and dialogs, table rows
            </span>
          </span>
          <div className={styles.samples}>
            {sizes.map((item) => (
              <div key={item.size} className={styles.sample}>
                <Button kind={item.kind} size={item.size}>
                  {item.label}
                </Button>
                <span className={styles.caption}>{item.name}</span>
              </div>
            ))}
          </div>
        </li>
        <li className={styles.card}>
          <span className={styles.name}>
            Busy on press{" "}
            <span className={styles.use}>· for {busyMs / 1000} seconds</span>
          </span>
          <div className={styles.samples}>
            <SaveSample />
          </div>
        </li>
      </ul>
    </section>
  );
}

function SaveSample() {
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!busy) return;
    const timer = setTimeout(() => setBusy(false), busyMs);
    return () => clearTimeout(timer);
  }, [busy]);

  return (
    <Button
      kind="primary"
      busy={busy}
      busyLabel="Saving"
      onClick={() => setBusy(true)}
    >
      Save
    </Button>
  );
}
