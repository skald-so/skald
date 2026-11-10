import { useState } from "react";
import { Button } from "../ui/Button.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { Loading } from "../ui/Loading.tsx";
import { sortRows, type Sort } from "../ui/sort.ts";
import { StatusDot } from "../ui/Status.tsx";
import {
  Mono,
  Table,
  TwoLine,
  type Column,
  type TableProps,
} from "../ui/Table.tsx";
import { accountColumns, accounts, accountValues } from "./accounts.tsx";
import styles from "./KitPage.module.css";

// Fixture rows of 08-machines.png and 07-queues.png; the accounts are in accounts.tsx.

interface Device {
  name: string;
  current?: boolean;
  kind: string;
  /** Minutes since the device was last seen; 0 is online. */
  seen: number;
  key: string;
  verified: "github" | "device" | "no";
}

const devices: Device[] = [
  {
    name: "ana-laptop",
    kind: "Machine · macOS",
    seen: 0,
    key: "7F3A 91C2 0B5E 44D8",
    verified: "github",
  },
  {
    name: "Chrome",
    current: true,
    kind: "Browser · macOS",
    seen: 0,
    key: "E2C7 0A9D 3B16 F4A2",
    verified: "device",
  },
  {
    name: "ana-cloud",
    kind: "Machine · Linux sandbox",
    seen: 180,
    key: "A06E 13FB 72C9 5E01",
    verified: "device",
  },
  {
    name: "ana-desktop",
    kind: "Machine · Windows",
    seen: 1440,
    key: "19BE 4F02 D7A3 C680",
    verified: "no",
  },
];

const verification = {
  github: "Verified with your GitHub SSH key",
  device: "Approved by ana-laptop",
  no: "Not verified · compare keys",
};

function seen(minutes: number): string {
  if (minutes === 0) return "Online";
  if (minutes < 1440) return `${minutes / 60} h ago`;
  return `${minutes / 1440} day ago`;
}

const deviceColumns: Column<Device>[] = [
  {
    key: "device",
    label: "Device",
    sortable: true,
    width: "minmax(0, 1.6fr)",
    cell: (device) => (
      <TwoLine detail={device.current && "this browser"}>{device.name}</TwoLine>
    ),
  },
  {
    key: "kind",
    label: "Kind",
    sortable: true,
    width: "minmax(0, 1.2fr)",
    muted: true,
    cell: (device) => device.kind,
  },
  {
    key: "seen",
    label: "Last seen",
    sortable: true,
    cell: (device) => (
      <StatusDot tone={device.seen === 0 ? "ok" : "muted"}>
        {seen(device.seen)}
      </StatusDot>
    ),
  },
  {
    key: "key",
    label: "Key",
    width: "minmax(0, 2fr)",
    cell: (device) => (
      <TwoLine
        mono
        detail={verification[device.verified]}
        detailTone={device.verified === "no" ? "warning" : "muted"}
      >
        {device.key}
      </TwoLine>
    ),
  },
  {
    key: "actions",
    label: "Actions",
    actions: true,
    width: "96px",
    cell: () => (
      <Button kind="danger" size="small">
        Revoke
      </Button>
    ),
  },
];

const deviceValues: Record<string, (device: Device) => string | number> = {
  device: (device) => device.name,
  kind: (device) => device.kind,
  // The latest first when descending.
  seen: (device) => -device.seen,
};

interface Queue {
  name: string;
  waiting: number;
  running: number;
  dead: number;
  /** Seconds the oldest task waits; none without waiting tasks. */
  oldest?: number;
}

const queues: Queue[] = [
  { name: "deploy", waiting: 0, running: 0, dead: 2 },
  { name: "research", waiting: 1, running: 2, dead: 0, oldest: 40 },
  { name: "review", waiting: 3, running: 1, dead: 0, oldest: 120 },
];

const queueColumns: Column<Queue>[] = [
  {
    key: "queue",
    label: "Queue",
    sortable: true,
    width: "minmax(0, 2fr)",
    cell: (queue) => <Mono>{queue.name}</Mono>,
  },
  {
    key: "waiting",
    label: "Waiting",
    sortable: true,
    cell: (queue) => queue.waiting,
  },
  {
    key: "running",
    label: "Running",
    sortable: true,
    cell: (queue) => queue.running,
  },
  {
    key: "dead",
    label: "Dead letters",
    sortable: true,
    emphasis: (queue) => (queue.dead > 0 ? "danger" : undefined),
    cell: (queue) => queue.dead,
  },
  {
    key: "oldest",
    label: "Oldest waiting",
    sortable: true,
    muted: true,
    cell: (queue) =>
      queue.oldest === undefined
        ? "—"
        : queue.oldest < 60
          ? `${queue.oldest} s`
          : `${queue.oldest / 60} min`,
  },
];

const queueValues: Record<string, (queue: Queue) => string | number> = {
  queue: (queue) => queue.name,
  waiting: (queue) => queue.waiting,
  running: (queue) => queue.running,
  dead: (queue) => queue.dead,
  oldest: (queue) => queue.oldest ?? -1,
};

/** A fixture table that sorts its rows in the browser, from `initial`. */
export function SortedTable<Row>({
  rows,
  values,
  initial,
  ...props
}: {
  rows: Row[];
  values: Record<string, (row: Row) => string | number>;
  initial: Sort;
} & Omit<TableProps<Row>, "rows" | "sort" | "onSort">) {
  const [sort, setSort] = useState(initial);
  return (
    <Table
      {...props}
      rows={sortRows(rows, sort, (row, column) => values[column]!(row))}
      sort={sort}
      onSort={setSort}
    />
  );
}

const noMachines = (
  <EmptyState
    action={
      <Button kind="primary" size="large">
        Connect a machine
      </Button>
    }
  >
    <strong>No machines yet.</strong> Connect one to send and receive messages
    from its agents.
  </EmptyState>
);

/** The surfaces of the design that hold lists: white cards and tinted ones, as the join request. */
const surfaces = [
  { name: "on a card", tinted: false },
  { name: "on a tinted card", tinted: true },
];

export function KitTables() {
  return (
    <section
      aria-labelledby="kit-tables"
      className={[styles.section, styles.wide].join(" ")}
    >
      <h2 id="kit-tables">Tables</h2>
      <div className={styles.tables}>
        <span className={styles.caption}>
          Two-line and key cells, an actions column · as Machines
        </span>
        <SortedTable
          label="Devices"
          columns={deviceColumns}
          rows={devices}
          rowKey={(device) => device.name}
          values={deviceValues}
          initial={{ column: "seen", direction: "descending" }}
          empty={noMachines}
          minWidth={760}
        />
        <span className={styles.caption}>
          A value that needs attention · as operator accounts
        </span>
        <SortedTable
          label="Accounts"
          columns={accountColumns}
          rows={accounts}
          rowKey={(account) => account.name}
          values={accountValues}
          initial={{ column: "messages", direction: "descending" }}
          empty={<EmptyState>No accounts match.</EmptyState>}
          minWidth={980}
        />
        <span className={styles.caption}>
          A highlighted row, ID cells · as Queues
        </span>
        <SortedTable
          label="All queues"
          columns={queueColumns}
          rows={queues}
          rowKey={(queue) => queue.name}
          values={queueValues}
          initial={{ column: "queue", direction: "ascending" }}
          highlight={(queue) => queue.dead > 0}
          empty={<EmptyState>No queues yet.</EmptyState>}
          minWidth={640}
        />
        <span className={styles.caption}>Empty</span>
        <SortedTable
          label="Devices, empty"
          columns={deviceColumns}
          rows={[]}
          rowKey={(device) => device.name}
          values={deviceValues}
          initial={{ column: "seen", direction: "descending" }}
          empty={noMachines}
          minWidth={760}
        />
        <span className={styles.caption}>Loading</span>
        <SortedTable
          label="Devices, loading"
          columns={deviceColumns}
          rows={devices}
          rowKey={(device) => device.name}
          values={deviceValues}
          initial={{ column: "seen", direction: "descending" }}
          loading
          loadingLabel="Loading devices"
          empty={noMachines}
          minWidth={760}
        />
      </div>

      <ul className={styles.cards}>
        {surfaces.flatMap((surface) =>
          [
            {
              state: "Empty",
              content: (
                <EmptyState action={<Button>Open the queues</Button>}>
                  No tasks are running.
                </EmptyState>
              ),
            },
            {
              state: "Loading",
              content: <Loading label="Loading tasks" lines={2} />,
            },
          ].map(({ state, content }) => (
            <li
              key={`${state} ${surface.name}`}
              className={[styles.card, surface.tinted && styles.tinted]
                .filter(Boolean)
                .join(" ")}
            >
              <span className={styles.name}>
                {state} <span className={styles.use}>· {surface.name}</span>
              </span>
              {content}
            </li>
          )),
        )}
      </ul>
    </section>
  );
}
