import { useEffect, useState } from "react";
import {
  endAccountSession,
  endOtherSessions,
  listServiceTokens,
  listSessions,
  revokeServiceToken,
} from "../api/skald.ts";
import type {
  ListedSession,
  ListedSpace,
  ServiceToken,
} from "../api/skald.schemas.ts";
import { Banner } from "../ui/Banner.tsx";
import { Button } from "../ui/Button.tsx";
import { ConfirmDialog } from "../ui/ConfirmDialog.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import { sortRows, type Sort } from "../ui/sort.ts";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import styles from "./Access.module.css";
import { signedIn, versioned } from "./api.ts";
import { InviteLinks } from "./InviteLinks.tsx";
import { useInviteLinks, type InviteLinksData } from "./invites.ts";
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

/** A live token of a service agent of `space`, a space where the account is an admin. */
export interface SpaceToken {
  space: ListedSpace;
  token: ServiceToken;
}

/** The service agent of `token`, as `ci/deploy-bot`. */
function agentOf({ token }: SpaceToken): string {
  return `${token.owner}/${token.agent}`;
}

/** The value of `row` in the sorted column `column` of the tokens. */
function tokenValue(row: SpaceToken, column: string): string | number {
  switch (column) {
    case "agent":
      return agentOf(row);
    case "created":
      return Date.parse(row.token.created_at);
    case "last_used":
      return row.token.last_used_at ? Date.parse(row.token.last_used_at) : 0;
    default:
      return Date.parse(row.token.expires_at);
  }
}

/** The number of `channels`, as "1 channel" or "2 channels". */
function channelCount(channels: readonly string[]): string {
  return channels.length === 1 ? "1 channel" : `${channels.length} channels`;
}

export interface AccessScreenProps {
  /** The account's sessions; none while they load. */
  sessions?: readonly ListedSession[];
  /**
   * The live tokens of the service agents of the spaces where the account is an admin; none while
   * they load or when it is an admin of no space, which hides them.
   */
  tokens?: readonly SpaceToken[];
  invites?: InviteLinksData;
  /** The time the sessions' relative times count from, in milliseconds since the epoch. */
  now: number;
  /** The time the tokens' relative times count from, as `now`; `now` without it. */
  tokensNow?: number;
  /** What failed, shown above the table. */
  failed?: string;
  /** Revoke on a session's row: ends it, without asking. */
  onRevoke: (session: ListedSession) => void;
  /** Confirmed "Sign out everywhere else": ends every other session. */
  onEndOthers: () => Promise<void>;
  /** Confirmed Revoke on a token's row: ends that token. */
  onRevokeToken: (token: SpaceToken) => Promise<void>;
  /** Opens the question of "Sign out everywhere else" on the first render, as on the kit page. */
  defaultAsking?: boolean;
}

/**
 * The Access section of 10-access.png: a row per session of the account, web or CLI, where and when
 * it signed in, its last use and expiry; Revoke on each but the page's own; and "Sign out everywhere
 * else", after a question. Under them, as the session rows, a row per live token of the service
 * agents of the spaces where the account is an admin, with Revoke after a question.
 */
export function AccessScreen({
  sessions,
  tokens,
  invites,
  now,
  tokensNow = now,
  failed,
  onRevoke,
  onEndOthers,
  onRevokeToken,
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
      {invites && <InviteLinks {...invites} showSpace />}
      {tokens && (
        <Tokens tokens={tokens} now={tokensNow} onRevoke={onRevokeToken} />
      )}
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

/**
 * The tokens of the service agents of the spaces where the account is an admin, as the session rows:
 * the agent and its space, the permissions, channels, creation, last use and expiry; Revoke on each,
 * after a question.
 */
function Tokens({
  tokens,
  now,
  onRevoke,
}: {
  tokens: readonly SpaceToken[];
  now: number;
  onRevoke: (token: SpaceToken) => Promise<void>;
}) {
  const [sort, setSort] = useState<Sort>();
  const [asking, setAsking] = useState<SpaceToken>();
  const [revoking, setRevoking] = useState(false);

  const columns: Column<SpaceToken>[] = [
    {
      key: "agent",
      label: "Service agent",
      sortable: true,
      width: "minmax(0, 1.6fr)",
      cell: (row) => <TwoLine detail={row.space.name}>{agentOf(row)}</TwoLine>,
    },
    {
      key: "permissions",
      label: "Permissions",
      width: "minmax(0, 1.2fr)",
      cell: ({ token }) => token.permissions.join(", "),
    },
    {
      key: "channels",
      label: "Channels",
      cell: ({ token }) => channelCount(token.channels),
    },
    {
      key: "created",
      label: "Created",
      sortable: true,
      cell: ({ token }) => date(token.created_at, now),
    },
    {
      key: "last_used",
      label: "Last used",
      sortable: true,
      cell: ({ token }) =>
        token.last_used_at ? ago(token.last_used_at, now) : "Never",
    },
    {
      key: "expires",
      label: "Expires",
      sortable: true,
      cell: ({ token }) => until(token.expires_at, now),
    },
    {
      key: "actions",
      label: "Actions",
      actions: true,
      width: "96px",
      cell: (row) => (
        <Button
          kind="danger"
          size="small"
          aria-label={`Revoke the token of ${agentOf(row)} in ${row.space.name}`}
          onClick={() => setAsking(row)}
        >
          Revoke
        </Button>
      ),
    },
  ];

  return (
    <div className={styles.tokens}>
      <div className={styles.heading}>
        <h2>Service agent tokens</h2>
        <p>
          Tokens of the service agents in the spaces you administer, made with{" "}
          <code>skald create-service-agent</code>. A revoked token is refused at
          once.
        </p>
      </div>
      {tokens.length === 0 ? (
        <EmptyState>No service agent tokens.</EmptyState>
      ) : (
        <Table
          label="Service agent tokens"
          columns={columns}
          rows={sort ? sortRows(tokens, sort, tokenValue) : tokens}
          rowKey={({ token }) => token.id}
          sort={sort}
          onSort={setSort}
          empty={null}
          minWidth={860}
        />
      )}
      {asking && (
        <ConfirmDialog
          title="Revoke this token?"
          confirm="Revoke token"
          busy={revoking}
          onCancel={() => setAsking(undefined)}
          onConfirm={() => {
            setRevoking(true);
            onRevoke(asking).finally(() => {
              setRevoking(false);
              setAsking(undefined);
            });
          }}
        >
          <p>
            The token of <code>{agentOf(asking)}</code> in {asking.space.name}{" "}
            stops working at once: the server refuses its next request. Make a
            new one with <code>skald create-service-token</code>.
          </p>
        </ConfirmDialog>
      )}
    </div>
  );
}

/** The account's sessions, or `null` when the API refuses the session; throws when it gives neither. */
async function fetchSessions(
  signal: AbortSignal,
): Promise<ListedSession[] | null> {
  const listed = signedIn(
    await listSessions({ headers: versioned, signal }),
    "GET /account/sessions",
  );
  return listed && listed.sessions;
}

/**
 * The live tokens of the service agents of those of `spaces` where the account is an admin, none
 * when it is an admin of none, or `null` when the API refuses the session; throws when it gives
 * neither. A space that refuses the list, as one where the account is no admin or that has no
 * revision yet, has none.
 */
async function fetchTokens(
  spaces: readonly ListedSpace[],
  signal: AbortSignal,
): Promise<SpaceToken[] | undefined | null> {
  const answers = await Promise.all(
    spaces.map((space) =>
      listServiceTokens(space.space, { headers: versioned, signal }),
    ),
  );
  if (answers.some((answer) => answer.status === 401)) return null;
  const found: SpaceToken[] = [];
  let admin = false;
  answers.forEach((answer, at) => {
    if (answer.status === 403 || answer.status === 404) return;
    if (answer.status !== 200)
      throw new Error(
        `GET /spaces/${spaces[at]!.space}/service-tokens ${answer.status}`,
      );
    admin = true;
    for (const token of answer.data.tokens)
      found.push({ space: spaces[at]!, token });
  });
  return admin ? found : undefined;
}

/**
 * The Access section with the account's sessions, and the tokens of the service agents of those of
 * `spaces` where it is an admin, from the API, listed again after each change. When the API refuses
 * the session, `onSignedOut` shows the sign-in page.
 */
export function Access({
  spaces,
  onSignedOut,
}: {
  spaces: readonly ListedSpace[];
  onSignedOut: () => void;
}) {
  const invites = useInviteLinks(spaces, onSignedOut);
  const [listed, setListed] = useState<{
    sessions: ListedSession[];
    at: number;
  }>();
  const [tokens, setTokens] = useState<{
    tokens: SpaceToken[] | undefined;
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

  useEffect(() => {
    const request = new AbortController();
    fetchTokens(spaces, request.signal).then(
      (found) =>
        found === null
          ? onSignedOut()
          : setTokens({ tokens: found, at: Date.now() }),
      () => {
        if (!request.signal.aborted)
          setFailed("The service agent tokens did not load.");
      },
    );
    return () => request.abort();
  }, [changes, spaces, onSignedOut]);

  /**
   * Runs `request`, which ends sessions or a token, then lists them again; shows `what` failed if it
   * did.
   */
  async function end(what: string, request: () => Promise<{ status: number }>) {
    setFailed(undefined);
    try {
      const { status } = await request();
      if (status === 401) return onSignedOut();
      // A session or token ended meanwhile is gone too.
      if (status !== 204 && status !== 404) setFailed(`${what} failed.`);
    } catch {
      setFailed(`${what} failed.`);
    }
    setChanges((changes) => changes + 1);
  }

  return (
    <AccessScreen
      sessions={listed?.sessions}
      tokens={tokens?.tokens}
      invites={invites}
      now={listed?.at ?? 0}
      tokensNow={tokens?.at}
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
      onRevokeToken={({ space, token }) =>
        end("Revoking the token", () =>
          revokeServiceToken(space.space, token.id, { headers: versioned }),
        )
      }
    />
  );
}
