import { Button } from "../ui/Button.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { StatusDot } from "../ui/Status.tsx";
import { TwoLine, type Column } from "../ui/Table.tsx";

// Fixture devices of 08-machines.png, for the tables and the cabinet.

export interface Device {
  name: string;
  current?: boolean;
  kind: string;
  /** Minutes since the device was last seen; 0 is online. */
  seen: number;
  key: string;
  verified: "github" | "device" | "no";
}

export const devices: Device[] = [
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

export const deviceColumns: Column<Device>[] = [
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

export const deviceValues: Record<string, (device: Device) => string | number> =
  {
    device: (device) => device.name,
    kind: (device) => device.kind,
    // The latest first when descending.
    seen: (device) => -device.seen,
  };

export const noMachines = (
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
