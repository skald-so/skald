import { useState } from "react";
import { Button } from "../ui/Button.tsx";
import { Dropdown, type DropdownOption } from "../ui/Dropdown.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { FilterBar } from "../ui/FilterBar.tsx";
import { StatusDot, type Tone } from "../ui/Status.tsx";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import styles from "./KitPage.module.css";

// The filter bar of 04-messages.png over fixture messages of that screen and the day before.

type Agent = "claude-code" | "codex";
type Delivery = "read" | "delivered" | "stored";
type Kind = "message" | "task";
type Day = "today" | "yesterday";

interface Message {
  id: string;
  agent: Agent;
  type: Kind;
  status: Delivery;
  day: Day;
  /** Minutes since midnight. */
  time: number;
  text: string;
}

const messages: Message[] = [
  {
    id: "m1",
    agent: "codex",
    type: "message",
    status: "read",
    day: "yesterday",
    time: 16 * 60 + 20,
    text: "Merged PR #127: the session cookie is now SameSite=Strict.",
  },
  {
    id: "m2",
    agent: "claude-code",
    type: "task",
    status: "delivered",
    day: "yesterday",
    time: 17 * 60 + 5,
    text: "Task deploy-7: roll out skald-api 1.4 to staging.",
  },
  {
    id: "m3",
    agent: "claude-code",
    type: "task",
    status: "read",
    day: "today",
    time: 10 * 60 + 42,
    text: "Review PR #128 in skald-api. Focus on token refresh and error handling.",
  },
  {
    id: "m4",
    agent: "codex",
    type: "message",
    status: "read",
    day: "today",
    time: 10 * 60 + 44,
    text: "Took review-42. Two issues so far: the refresh token is logged at debug level, and a 401 from the IdP is retried forever.",
  },
  {
    id: "m5",
    agent: "claude-code",
    type: "message",
    status: "delivered",
    day: "today",
    time: 10 * 60 + 46,
    text: "Fix the logging issue in this PR. For the retries, propose a limit in a separate PR.",
  },
  {
    id: "m6",
    agent: "codex",
    type: "message",
    status: "stored",
    day: "today",
    time: 10 * 60 + 58,
    text: "Done: PR #131 removes the token from the logs. The retry limit proposal is in PR #132.",
  },
];

const machines: Record<Agent, string> = {
  "claude-code": "ana-laptop · Ana",
  codex: "build-01 · Lee",
};

const deliveries: Record<Delivery, { label: string; tone: Tone }> = {
  read: { label: "Read", tone: "ok" },
  delivered: { label: "Delivered", tone: "muted" },
  stored: { label: "Stored", tone: "warning" },
};

const clock = (minutes: number) =>
  `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;

const columns: Column<Message>[] = [
  {
    key: "from",
    label: "From",
    width: "minmax(0, 1.2fr)",
    cell: (message) => (
      <TwoLine detail={machines[message.agent]}>{message.agent}</TwoLine>
    ),
  },
  {
    key: "message",
    label: "Message",
    width: "minmax(0, 3fr)",
    cell: (message) => message.text,
  },
  {
    key: "type",
    label: "Type",
    width: "80px",
    muted: true,
    cell: (message) => (message.type === "task" ? "Task" : "Message"),
  },
  {
    key: "status",
    label: "Status",
    width: "110px",
    cell: (message) => (
      <StatusDot tone={deliveries[message.status].tone}>
        {deliveries[message.status].label}
      </StatusDot>
    ),
  },
  {
    key: "time",
    label: "Time",
    width: "120px",
    muted: true,
    cell: (message) =>
      `${message.day === "today" ? "Today" : "Yesterday"} ${clock(message.time)}`,
  },
];

type From = "anyone" | Agent;
type Status = "any" | Delivery;
type Type = "all" | Kind;
type Time = "any" | Day;
type Order = "oldest" | "newest";

const fromOptions: DropdownOption<From>[] = [
  { value: "anyone", label: "anyone" },
  { value: "claude-code", label: "claude-code" },
  { value: "codex", label: "codex" },
];

const statusOptions: DropdownOption<Status>[] = [
  { value: "any", label: "any" },
  { value: "read", label: "read" },
  { value: "delivered", label: "delivered" },
  { value: "stored", label: "stored" },
];

const typeOptions: DropdownOption<Type>[] = [
  { value: "all", label: "messages and tasks" },
  { value: "message", label: "messages" },
  { value: "task", label: "tasks" },
];

const timeOptions: DropdownOption<Time>[] = [
  { value: "any", label: "any time" },
  { value: "today", label: "today" },
  { value: "yesterday", label: "yesterday" },
];

const orderOptions: DropdownOption<Order>[] = [
  { value: "oldest", label: "Oldest first" },
  { value: "newest", label: "Newest first" },
];

interface Filters {
  from: From;
  status: Status;
  type: Type;
  time: Time;
}

const cleared: Filters = {
  from: "anyone",
  status: "any",
  type: "all",
  time: "any",
};

const day = { yesterday: 0, today: 1 };

/** The fixture messages in the filters, in the order. */
function shown(filters: Filters, order: Order): Message[] {
  const rows = messages.filter(
    (message) =>
      (filters.from === "anyone" || message.agent === filters.from) &&
      (filters.status === "any" || message.status === filters.status) &&
      (filters.type === "all" || message.type === filters.type) &&
      (filters.time === "any" || message.day === filters.time),
  );
  const sign = order === "oldest" ? 1 : -1;
  return rows.sort(
    (a, b) => sign * (day[a.day] - day[b.day] || a.time - b.time),
  );
}

export function KitDropdowns() {
  return (
    <section
      aria-labelledby="kit-dropdowns"
      className={[styles.section, styles.wide].join(" ")}
    >
      <h2 id="kit-dropdowns">Dropdown filters</h2>
      <FilteredMessages />
      <ul className={styles.cards}>
        <li className={[styles.card, styles.open].join(" ")}>
          <span className={styles.name}>
            Open <span className={styles.use}>· a set filter</span>
          </span>
          <OpenSample />
        </li>
      </ul>
    </section>
  );
}

function FilteredMessages() {
  // As 04-messages.png: today's messages, oldest first.
  const [filters, setFilters] = useState<Filters>({
    ...cleared,
    time: "today",
  });
  const [order, setOrder] = useState<Order>("oldest");
  const set = (change: Partial<Filters>) =>
    setFilters((filters) => ({ ...filters, ...change }));
  const filtered = (Object.keys(cleared) as (keyof Filters)[]).some(
    (key) => filters[key] !== cleared[key],
  );

  return (
    <div className={styles.tables}>
      <span className={styles.caption}>
        Narrowing a table · as the filter bar of Messages
      </span>
      <FilterBar
        label="Filter messages"
        filtered={filtered}
        onClear={() => setFilters(cleared)}
        sort={
          <Dropdown
            label="Sort"
            options={orderOptions}
            value={order}
            defaultValue="oldest"
            onChange={setOrder}
          />
        }
      >
        <Dropdown
          label="From"
          options={fromOptions}
          value={filters.from}
          defaultValue={cleared.from}
          onChange={(from) => set({ from })}
        />
        <Dropdown
          label="Status"
          options={statusOptions}
          value={filters.status}
          defaultValue={cleared.status}
          onChange={(status) => set({ status })}
        />
        <Dropdown
          label="Type"
          options={typeOptions}
          value={filters.type}
          defaultValue={cleared.type}
          onChange={(type) => set({ type })}
        />
        <Dropdown
          label="Time"
          options={timeOptions}
          value={filters.time}
          defaultValue={cleared.time}
          onChange={(time) => set({ time })}
        />
      </FilterBar>
      <Table
        label="Filtered messages"
        columns={columns}
        rows={shown(filters, order)}
        rowKey={(message) => message.id}
        empty={
          <EmptyState
            action={
              <Button onClick={() => setFilters(cleared)}>Clear filters</Button>
            }
          >
            No messages match.
          </EmptyState>
        }
        minWidth={820}
      />
    </div>
  );
}

function OpenSample() {
  const [type, setType] = useState<Type>("task");
  return (
    <div>
      <Dropdown
        label="Type"
        options={typeOptions}
        value={type}
        defaultValue="all"
        onChange={setType}
        defaultOpen
      />
    </div>
  );
}
