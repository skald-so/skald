import { Button } from "../ui/Button.tsx";
import { StatusLabel } from "../ui/Status.tsx";
import { TwoLine, type Column } from "../ui/Table.tsx";

// Fixture accounts of 13-operator-accounts.png: the tables and the filters of the kit page.

type AccountStatus = "Active" | "Flagged" | "Banned";

export interface Account {
  name: string;
  method: string;
  handle: string;
  /** Day of 2026, as `08-01`. */
  created: string;
  spaces: number;
  devices: number;
  messages: number;
  plan?: string;
  status: AccountStatus;
  /** Sends unusually many messages. */
  hot?: boolean;
}

export const accounts: Account[] = [
  {
    name: "Northwind",
    method: "SSO",
    handle: "northwind.io",
    created: "08-01",
    spaces: 12,
    devices: 140,
    messages: 6_100_000,
    plan: "Enterprise",
    status: "Active",
  },
  {
    name: "spam-relay",
    method: "GitHub",
    handle: "spam-relay",
    created: "11-09",
    spaces: 3,
    devices: 10,
    messages: 980_000,
    status: "Flagged",
    hot: true,
  },
  {
    name: "Acme CI",
    method: "Google",
    handle: "ci@acme.dev",
    created: "09-14",
    spaces: 3,
    devices: 10,
    messages: 920_000,
    status: "Active",
    hot: true,
  },
  {
    name: "Ana Kovac",
    method: "GitHub",
    handle: "ana",
    created: "10-02",
    spaces: 1,
    devices: 4,
    messages: 12_000,
    status: "Active",
  },
  {
    name: "Lee Park",
    method: "Google",
    handle: "lee.park@gmail.com",
    created: "11-04",
    spaces: 1,
    devices: 2,
    messages: 3_000,
    status: "Active",
  },
  {
    name: "free-tokens",
    method: "GitHub",
    handle: "free-tokens",
    created: "10-30",
    spaces: 3,
    devices: 9,
    messages: 0,
    status: "Banned",
  },
];

const months = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split(" ");

function day(created: string): string {
  const [month, date] = created.split("-").map(Number);
  return `${months[month! - 1]} ${date}`;
}

function messages(count: number): string {
  if (count >= 1_000_000) return `${count / 1_000_000}M`;
  if (count >= 1_000) return `${count / 1_000}k`;
  return String(count);
}

const statusTone = {
  Active: "ok",
  Flagged: "warning",
  Banned: "danger",
} as const;

export const accountColumns: Column<Account>[] = [
  {
    key: "account",
    label: "Account",
    sortable: true,
    width: "minmax(0, 2fr)",
    cell: (account) => (
      <TwoLine detail={`${account.method} · ${account.handle}`}>
        {account.name}
      </TwoLine>
    ),
  },
  {
    key: "created",
    label: "Created",
    sortable: true,
    width: "minmax(0, 0.8fr)",
    muted: true,
    cell: (account) => day(account.created),
  },
  {
    key: "spaces",
    label: "Spaces",
    sortable: true,
    width: "minmax(0, 0.8fr)",
    cell: (account) => account.spaces,
  },
  {
    key: "devices",
    label: "Devices",
    sortable: true,
    width: "minmax(0, 0.8fr)",
    cell: (account) => account.devices,
  },
  {
    key: "messages",
    label: "Msgs / month",
    sortable: true,
    width: "minmax(0, 0.8fr)",
    emphasis: (account) => (account.hot ? "warning" : undefined),
    cell: (account) => messages(account.messages),
  },
  {
    key: "plan",
    label: "Plan",
    sortable: true,
    muted: true,
    cell: (account) => account.plan ?? "—",
  },
  {
    key: "status",
    label: "Status",
    sortable: true,
    width: "minmax(0, 0.9fr)",
    cell: (account) => (
      <StatusLabel tone={statusTone[account.status]}>
        {account.status}
      </StatusLabel>
    ),
  },
  {
    key: "actions",
    label: "Actions",
    actions: true,
    width: "120px",
    cell: () => <Button size="small">Manage</Button>,
  },
];

export const accountValues: Record<
  string,
  (account: Account) => string | number
> = {
  account: (account) => account.name,
  created: (account) => account.created,
  spaces: (account) => account.spaces,
  devices: (account) => account.devices,
  messages: (account) => account.messages,
  plan: (account) => account.plan ?? "",
  status: (account) => account.status,
};
