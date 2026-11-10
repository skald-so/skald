import {
  CountBadge,
  StatusDot,
  StatusLabel,
  type Tone,
} from "../ui/Status.tsx";
import styles from "./KitPage.module.css";

const tones: {
  tone: Tone;
  name: string;
  label: string;
  dot: string;
  count: number;
  countLabel: string;
}[] = [
  {
    tone: "ok",
    name: "OK",
    label: "Active",
    dot: "Online",
    count: 3,
    countLabel: "online",
  },
  {
    tone: "warning",
    name: "Warning",
    label: "Flagged",
    dot: "Not verified",
    count: 2,
    countLabel: "in dead letters",
  },
  {
    tone: "danger",
    name: "Danger",
    label: "Banned",
    dot: "Expired",
    count: 1,
    countLabel: "failed",
  },
  {
    tone: "accent",
    name: "Accent",
    label: "Limit request",
    dot: "Syncing",
    count: 2,
    countLabel: "unread",
  },
  {
    tone: "muted",
    name: "Muted",
    label: "Revoked",
    dot: "3 h ago",
    count: 0,
    countLabel: "waiting",
  },
];

export function KitStatuses() {
  return (
    <section aria-labelledby="kit-statuses" className={styles.section}>
      <h2 id="kit-statuses">Status labels</h2>
      <table className={styles.statuses}>
        <thead>
          <tr>
            <th scope="col">Tone</th>
            <th scope="col">Filled label</th>
            <th scope="col">Dot with text</th>
            <th scope="col">Count badge</th>
          </tr>
        </thead>
        <tbody>
          {tones.map((item) => (
            <tr key={item.tone}>
              <th scope="row">{item.name}</th>
              <td>
                <StatusLabel tone={item.tone}>{item.label}</StatusLabel>
              </td>
              <td>
                <StatusDot tone={item.tone}>{item.dot}</StatusDot>
              </td>
              <td>
                <CountBadge
                  tone={item.tone}
                  count={item.count}
                  label={item.countLabel}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
