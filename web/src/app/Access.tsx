import { useEffect, useState } from "react";
import {
  endAccountSession,
  endOtherSessions,
  listSessions,
} from "../api/skald.ts";
import type { ListedSession } from "../api/skald.schemas.ts";
import { Banner } from "../ui/Banner.tsx";
import { Button } from "../ui/Button.tsx";
import { ConfirmDialog } from "../ui/ConfirmDialog.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import { sortRows, type Sort } from "../ui/sort.ts";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import { versioned } from "./api.ts";
import { ago, date, until } from "./time.ts";

/** What follows "skald" for a CLI session: its device. */
function device(session: ListedSession): string {
  return session.device_name ? ` on ${session.device_name}` : ", no device yet";
}

/** Where a session signed in: a web session's browser and OS, a CLI session's device. */
function where(session: ListedSession): string {
  if (session.kind === "web")
    return `${session.browser ?? "Unknown browser"} · ${session.os ?? "unknown OS"}`;
  return `skald${device(session)}`;
}

/** The value of `session` in the sorted column `column`. */
function value(session: ListedSession, column: string): string | number {
  switch (column) {
    case "type":
      return session.kind;
    case "created":
      return Date.parse(session.started_at);
    case "expires":
      return session.expires_at
        ? Date.parse(session.expires_at)
        : Number.MAX_SAFE_INTEGER;
    default:
      return Date.parse(session.last_used_at);
  }
}

export interface AccessScreenProps {
  /** The account's sessions; none while they load. */
  sessions?: readonly ListedSession[];
  /** The time the relative times count from, in milliseconds since the epoch. */
  now: number;
  /** What failed, shown above the table. */
  failed?: string;
  /** Revoke on a session's row: ends it, without asking. */
  onRevoke: (session: ListedSession) => void;
  /** Confirmed "Sign out everywhere else": ends every other session. */
  onEndOthers: () => Promise<void>;
  /** Opens the question of "Sign out everywhere else" on the first render, as on the kit page. */
  defaultAsking?: boolean;
}

/**
 * The Access section of 10-access.png: a row per session of the account, web or CLI, where and when
 * it signed in, its last use and expiry; Revoke on each but the page's own; and "Sign out everywhere
 * else", after a question.
 */
export function AccessScreen({
  sessions,
  now,
  failed,
  onRevoke,
  onEndOthers,
  defaultAsking = false,
}: AccessScreenProps) {
  const [sort, setSort] = useState<Sort>({
    column: "last_used",
    direction: "descending",
  });
  const [asking, setAsking] = useState(defaultAsking);
  const [ending, setEnding] = useState(false);

  const columns: Column<ListedSession>[] = [
    {
      key: "type",
      label: "Type",
      sortable: true,
      width: "minmax(0, 0.8fr)",
      cell: (session) => (
        <TwoLine>{session.kind === "web" ? "Web" : "CLI"}</TwoLine>
      ),
    },
    {
      key: "where",
      label: "Where",
      width: "minmax(0, 2fr)",
      cell: (session) => (
        <TwoLine
          regular
          detail={session.current && "this session"}
          detailTone="ok"
        >
          {session.kind === "device" ? (
            <>
              <code>skald</code>
              {device(session)}
            </>
          ) : (
            where(session)
          )}
        </TwoLine>
      ),
    },
    {
      key: "created",
      label: "Created",
      sortable: true,
      cell: (session) => date(session.started_at, now),
    },
    {
      key: "last_used",
      label: "Last used",
      sortable: true,
      cell: (session) => ago(session.last_used_at, now),
    },
    {
      key: "expires",
      label: "Expires",
      sortable: true,
      cell: (session) =>
        session.expires_at ? until(session.expires_at, now) : "when revoked",
    },
    {
      key: "actions",
      label: "Actions",
      actions: true,
      width: "96px",
      cell: (session) =>
        !session.current && (
          <Button
            kind="danger"
            size="small"
            aria-label={`Revoke ${where(session)}`}
            onClick={() => onRevoke(session)}
          >
            Revoke
          </Button>
        ),
    },
  ];

  return (
    <>
      <PageHeader
        title="Access"
        subtitle="Where your account is signed in. A revoked session loses access at once."
        actions={
          <Button kind="danger" size="large" onClick={() => setAsking(true)}>
            Sign out everywhere else
          </Button>
        }
      />
      {failed && <Banner>{failed}</Banner>}
      <Table
        label="Sessions"
        columns={columns}
        rows={sessions ? sortRows(sessions, sort, value) : []}
        rowKey={(session) => session.session}
        sort={sort}
        onSort={setSort}
        loading={!sessions}
        loadingLabel="Loading sessions"
        empty={<EmptyState>No sessions.</EmptyState>}
        minWidth={720}
      />
      {asking && (
        <ConfirmDialog
          title="Sign out everywhere else?"
          confirm="Sign out everywhere else"
          busy={ending}
          onCancel={() => setAsking(false)}
          onConfirm={() => {
            setEnding(true);
            onEndOthers().finally(() => {
              setEnding(false);
              setAsking(false);
            });
          }}
        >
          <p>
            Every other session of your account ends: your other browsers, and
            the machines signed in with <code>skald</code>, which sign in again
            to resume. This browser stays signed in.
          </p>
        </ConfirmDialog>
      )}
    </>
  );
}

/** The account's sessions, or `null` when the API refuses the session; throws when it gives neither. */
async function fetchSessions(
  signal: AbortSignal,
): Promise<ListedSession[] | null> {
  const listed = await listSessions({ headers: versioned, signal });
  if (listed.status === 401) return null;
  if (listed.status !== 200)
    throw new Error(`GET /account/sessions ${listed.status}`);
  return listed.data.sessions;
}

/**
 * The Access section with the account's sessions from the API, listed again after each change.
 * When the API refuses the session, `onSignedOut` shows the sign-in page.
 */
export function Access({ onSignedOut }: { onSignedOut: () => void }) {
  const [listed, setListed] = useState<{
    sessions: ListedSession[];
    at: number;
  }>();
  const [failed, setFailed] = useState<string>();
  /** Counts the changes, each of which lists the sessions again. */
  const [changes, setChanges] = useState(0);

  useEffect(() => {
    const request = new AbortController();
    fetchSessions(request.signal).then(
      (sessions) =>
        sessions ? setListed({ sessions, at: Date.now() }) : onSignedOut(),
      () => {
        if (!request.signal.aborted) setFailed("The sessions did not load.");
      },
    );
    return () => request.abort();
  }, [changes, onSignedOut]);

  /** Runs `request`, which ends sessions, then lists them again; shows `what` failed if it did. */
  async function end(what: string, request: () => Promise<{ status: number }>) {
    setFailed(undefined);
    try {
      const { status } = await request();
      if (status === 401) return onSignedOut();
      // A session ended meanwhile is gone too.
      if (status !== 204 && status !== 404) setFailed(`${what} failed.`);
    } catch {
      setFailed(`${what} failed.`);
    }
    setChanges((changes) => changes + 1);
  }

  return (
    <AccessScreen
      sessions={listed?.sessions}
      now={listed?.at ?? 0}
      failed={failed}
      onRevoke={(session) =>
        end("Revoking the session", () =>
          endAccountSession(session.session, { headers: versioned }),
        )
      }
      onEndOthers={() =>
        end("Signing out everywhere else", () =>
          endOtherSessions({ headers: versioned }),
        )
      }
    />
  );
}
