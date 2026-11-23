import { useId, useState } from "react";
import { Banner } from "../ui/Banner.tsx";
import { Button } from "../ui/Button.tsx";
import { ConfirmDialog } from "../ui/ConfirmDialog.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import type { InviteLinksData, SpaceInvite } from "./invites.ts";
import styles from "./InviteLinks.module.css";
import { date, until } from "./time.ts";

function role({ invite }: SpaceInvite) {
  return invite.role === "admin" ? "Admin" : "Member";
}

function waiting({ invite }: SpaceInvite) {
  if (!invite.acceptance_waiting) return "Waiting for newcomer";
  const state = "Waiting for an admin’s device";
  return invite.acceptance_handle
    ? `${state} (${invite.acceptance_handle})`
    : state;
}

/** Invite cards on Members and invite rows across spaces on Access. */
export function InviteLinks({
  rows,
  now,
  failed,
  revoke,
  showSpace = false,
}: InviteLinksData & { showSpace?: boolean }) {
  const heading = useId();
  const [asking, setAsking] = useState<SpaceInvite>();
  const [revoking, setRevoking] = useState(false);

  if (!rows) return failed && <Banner>{failed}</Banner>;

  const action = (row: SpaceInvite) => (
    <Button
      kind="danger"
      size="small"
      aria-label={`Revoke ${role(row)} invite link in ${row.space.name}${row.invite.device_name ? ` from ${row.invite.device_name}` : ""}`}
      onClick={() => setAsking(row)}
    >
      Revoke
    </Button>
  );
  const columns: Column<SpaceInvite>[] = [
    {
      key: "type",
      label: "Type",
      cell: ({ space }) => <TwoLine detail={space.name}>Invite link</TwoLine>,
    },
    { key: "role", label: "Role", cell: role },
    { key: "state", label: "State", width: "minmax(0, 2fr)", cell: waiting },
    {
      key: "created",
      label: "Created",
      muted: true,
      cell: ({ invite }) => date(invite.created_at, now),
    },
    {
      key: "expires",
      label: "Expires",
      muted: true,
      cell: ({ invite }) => until(invite.expires_at, now),
    },
    {
      key: "actions",
      label: "Actions",
      width: "96px",
      actions: true,
      cell: action,
    },
  ];

  return (
    <section className={styles.links} aria-labelledby={heading}>
      <h2 id={heading}>Invite links</h2>
      {failed && <Banner>{failed}</Banner>}
      {rows.length === 0 ? (
        <EmptyState>No invite links.</EmptyState>
      ) : showSpace ? (
        <Table
          label="Active invite links"
          columns={columns}
          rows={rows}
          rowKey={({ invite }) => invite.invite}
          minWidth={720}
          empty={null}
        />
      ) : (
        <ul className={styles.cards} role="list">
          {rows.map((row) => (
            <li className={styles.card} key={row.invite.invite}>
              <div className={styles.details}>
                <span>
                  Created {date(row.invite.created_at, now)} by{" "}
                  {row.invite.username ?? "a deleted account"}
                  {row.invite.device_name &&
                    ` on ${row.invite.device_name}`} · {role(row)}
                </span>
                <span>
                  {waiting(row)} · Expires {until(row.invite.expires_at, now)}
                </span>
              </div>
              {action(row)}
            </li>
          ))}
        </ul>
      )}
      {asking && (
        <ConfirmDialog
          title="Revoke this invite link?"
          confirm="Revoke link"
          busy={revoking}
          onCancel={() => {
            if (!revoking) setAsking(undefined);
          }}
          onConfirm={() => {
            setRevoking(true);
            revoke(asking).finally(() => {
              setAsking(undefined);
              setRevoking(false);
            });
          }}
        >
          <p>The link and any waiting acceptance will expire immediately.</p>
        </ConfirmDialog>
      )}
    </section>
  );
}
