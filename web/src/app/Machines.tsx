import { useEffect, useState } from "react";
import { listAccountDevices } from "../api/skald.ts";
import type {
  ListedDevice,
  ListedDeviceKind,
  ListedDeviceState,
} from "../api/skald.schemas.ts";
import { Banner } from "../ui/Banner.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import { sortRows, type Sort } from "../ui/sort.ts";
import { StatusDot, type Tone } from "../ui/Status.tsx";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import { signedIn, versioned } from "./api.ts";
import { date } from "./time.ts";

/** Each kind of device for people. */
const kinds: Record<ListedDeviceKind, string> = {
  client: "Machine",
  browser: "Browser",
  service: "Service agent",
};

/** Each state for people, its tone and its place when sorted. */
const states: Record<
  ListedDeviceState,
  { label: string; tone: Tone; order: number }
> = {
  linked: { label: "Linked", tone: "ok", order: 0 },
  waiting: { label: "Waiting for linking", tone: "warning", order: 1 },
  revoked: { label: "Revoked", tone: "danger", order: 2 },
};

/** The value of `device` in the sorted column `column`. */
function value(device: ListedDevice, column: string): string | number {
  switch (column) {
    case "kind":
      return kinds[device.kind];
    case "registered":
      return Date.parse(device.registered_at);
    case "status":
      return states[device.state].order;
    default:
      return device.name;
  }
}

export interface MachinesScreenProps {
  /** The account's username, which the header names. */
  username: string;
  /** The account's devices; none while they load. */
  devices?: readonly ListedDevice[];
  /** The time the dates count from, in milliseconds since the epoch. */
  now: number;
  /** What failed, shown above the table. */
  failed?: string;
}

/**
 * The Machines section of 08-machines.png: a row per device of the account with its name, kind,
 * when it registered and its state by the server's checks, in the server's order until a click on
 * a column sorts them.
 */
export function MachinesScreen({
  username,
  devices,
  now,
  failed,
}: MachinesScreenProps) {
  const [sort, setSort] = useState<Sort>();

  const columns: Column<ListedDevice>[] = [
    {
      key: "device",
      label: "Device",
      sortable: true,
      width: "minmax(0, 1.6fr)",
      cell: (device) => <TwoLine>{device.name}</TwoLine>,
    },
    {
      key: "kind",
      label: "Kind",
      sortable: true,
      muted: true,
      cell: (device) => kinds[device.kind],
    },
    {
      key: "registered",
      label: "Registered",
      sortable: true,
      cell: (device) => date(device.registered_at, now),
    },
    {
      key: "status",
      label: "Status",
      sortable: true,
      width: "minmax(0, 1.4fr)",
      cell: (device) => (
        <StatusDot tone={states[device.state].tone}>
          {states[device.state].label}
        </StatusDot>
      ),
    },
  ];

  return (
    <>
      <PageHeader
        title="Machines"
        subtitle={
          <>
            The devices of your account {username}, as the server lists them;{" "}
            <code>skald</code> checks them itself. A revoked device loses access
            to every channel.
          </>
        }
      />
      {failed && <Banner>{failed}</Banner>}
      <Table
        label="Devices"
        columns={columns}
        rows={devices ? (sort ? sortRows(devices, sort, value) : devices) : []}
        rowKey={(device) => device.device}
        sort={sort}
        onSort={setSort}
        loading={!devices}
        loadingLabel="Loading devices"
        empty={
          <EmptyState>
            No machines yet. Sign in a machine with <code>skald</code> to list
            it here.
          </EmptyState>
        }
        minWidth={560}
      />
    </>
  );
}

/** The account's devices, or `null` when the API refuses the session; throws when it gives neither. */
async function fetchDevices(
  signal: AbortSignal,
): Promise<ListedDevice[] | null> {
  const listed = signedIn(
    await listAccountDevices({ headers: versioned, signal }),
    "GET /account/devices",
  );
  return listed && listed.devices;
}

/**
 * The Machines section of `username`'s account with its devices from the API. When the API refuses
 * the session, `onSignedOut` shows the sign-in page.
 */
export function Machines({
  username,
  onSignedOut,
}: {
  username: string;
  onSignedOut: () => void;
}) {
  const [listed, setListed] = useState<{
    devices: ListedDevice[];
    at: number;
  }>();
  const [failed, setFailed] = useState<string>();

  useEffect(() => {
    const request = new AbortController();
    fetchDevices(request.signal).then(
      (devices) =>
        devices ? setListed({ devices, at: Date.now() }) : onSignedOut(),
      () => {
        if (!request.signal.aborted) setFailed("The devices did not load.");
      },
    );
    return () => request.abort();
  }, [onSignedOut]);

  return (
    <MachinesScreen
      username={username}
      devices={listed?.devices}
      now={listed?.at ?? 0}
      failed={failed}
    />
  );
}
