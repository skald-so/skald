import { useState } from "react";
import { Button } from "../ui/Button.tsx";
import { Dropdown, type DropdownOption } from "../ui/Dropdown.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import type { MenuAction } from "../ui/Menu.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import {
  AccountFooter,
  Shell,
  type ShellAccount,
  type ShellSection,
} from "../ui/Shell.tsx";
import styles from "./KitPage.module.css";
import {
  deviceColumns,
  devices,
  deviceValues,
  noMachines,
} from "./devices.tsx";
import { SortedTable } from "./KitTables.tsx";

// The cabinet of 08-machines.png with fixture sections, spaces and account.

const sections: ShellSection[] = [
  {
    key: "messages",
    label: "Messages",
    href: "#messages",
    items: [
      { key: "review", label: "# review", href: "#review" },
      {
        key: "deploy",
        label: "# deploy",
        href: "#deploy",
        badge: { count: 2, label: "unread", tone: "accent" },
      },
      { key: "research", label: "# research", href: "#research" },
    ],
  },
  {
    key: "queues",
    label: "Queues",
    href: "#queues",
    badge: { count: 2, label: "waiting", tone: "warning" },
  },
  { key: "machines", label: "Machines", href: "#machines" },
  { key: "members", label: "Members", href: "#members" },
  { key: "access", label: "Access", href: "#access" },
  { key: "usage", label: "Usage", href: "#usage" },
];

const spaces: DropdownOption<string>[] = [
  { value: "team-a", label: "team-a" },
  { value: "team-b", label: "team-b" },
  { value: "ana", label: "ana" },
];

const actions: MenuAction[] = [
  { label: "Sign-in methods", onSelect: () => {} },
  { label: "Sign out", onSelect: () => {} },
];

const account: ShellAccount = {
  name: "Ana Kovac",
  initials: "AK",
  signIn: "GitHub · ana",
  actions,
};

/** The heading of every fixture section; Machines also has its screen. */
function label(key: string): string {
  return sections
    .flatMap((section) => [section, ...(section.items ?? [])])
    .find((item) => item.key === key)!.label;
}

/**
 * The cabinet around the Machines screen, built from the kit components. Choosing a section marks
 * it current and shows its heading; `menuOpen` opens the side menu at phone width.
 */
export function KitCabinet({ menuOpen = false }: { menuOpen?: boolean }) {
  const [current, setCurrent] = useState("machines");
  const [space, setSpace] = useState("team-a");

  return (
    <Shell
      sections={sections}
      current={current}
      onNavigate={setCurrent}
      spaces={spaces}
      space={space}
      onSpaceChange={setSpace}
      account={account}
      home="kit.html"
      defaultMenuOpen={menuOpen}
    >
      {current === "machines" ? (
        <Machines />
      ) : (
        <>
          <PageHeader title={label(current)} />
          <EmptyState>The kit has no screen for this section.</EmptyState>
        </>
      )}
    </Shell>
  );
}

function Machines() {
  return (
    <>
      <PageHeader
        title="Machines"
        subtitle="Your devices. Each has its own key; a revoked device loses access to every channel."
        actions={
          <Button kind="primary" size="large">
            Connect a machine
          </Button>
        }
      />
      <section aria-label="Connect a machine" className={styles.connect}>
        <span className={styles.name}>Run on the new machine</span>
        <div className={styles.command}>
          <code>npx skald-so connect --code K7QM-4XWD</code>
          <Button
            size="small"
            aria-label="Copy command"
            className={styles.copy}
          >
            Copy
          </Button>
        </div>
        <span className={styles.note}>
          The code expires in 10 minutes. Claude Code and Codex on that machine
          are set up automatically.
        </span>
      </section>
      <section
        aria-label="New device waiting for approval"
        className={styles.request}
      >
        <div className={styles.requestText}>
          <span className={styles.name}>
            New device wants to join: ana-mini · macOS
          </span>
          <span className={styles.note}>
            Check that the machine shows the same key:{" "}
            <code className={styles.key}>5D2A 7C10 E8B4 39F1</code>
          </span>
        </div>
        <div className={styles.requestActions}>
          <Button kind="primary">Approve</Button>
          <Button>Reject</Button>
        </div>
      </section>
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
    </>
  );
}

/**
 * The cabinet on the kit page: links to the cabinet views, and the space switcher and the account
 * menu open, as they sit in the side menu.
 */
export function KitShell({ query }: { query: string }) {
  const [space, setSpace] = useState("team-b");
  return (
    <section aria-labelledby="kit-cabinet" className={styles.section}>
      <h2 id="kit-cabinet">Cabinet</h2>
      <ul className={styles.links}>
        <li>
          <a href={`kit.html?${query}screen=cabinet`}>
            The cabinet around Machines
          </a>
        </li>
        <li>
          <a href={`kit.html?${query}screen=cabinet-menu`}>
            The side menu open at phone width
          </a>
        </li>
      </ul>
      <div className={styles.samples}>
        <div className={styles.sample}>
          <div className={`${styles.side} ${styles.switcher}`}>
            <Dropdown
              stacked
              label="Space"
              options={spaces}
              value={space}
              onChange={setSpace}
              defaultOpen
            />
          </div>
          <span className={styles.caption}>Space switcher, open</span>
        </div>
        <div className={styles.sample}>
          <div className={`${styles.side} ${styles.accountMenu}`}>
            <AccountFooter account={account} defaultOpen />
          </div>
          <span className={styles.caption}>Account menu, open</span>
        </div>
      </div>
    </section>
  );
}
