import { useCallback, useEffect, useId, useState } from "react";
import {
  deleteAccount,
  getAccountDeletion,
  getSignInProviders,
  listSignInMethods,
  removeSignInMethod,
} from "../api/skald.ts";
import type {
  AccountDeletion,
  DeletionSpace,
  Session,
  SignInMethod,
  SignInProvider,
} from "../api/skald.schemas.ts";
import { Banner } from "../ui/Banner.tsx";
import { Button } from "../ui/Button.tsx";
import { ConfirmDialog } from "../ui/ConfirmDialog.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import styles from "./Account.module.css";
import { signedIn, versioned } from "./api.ts";
import { accountPath, providerNames } from "./names.ts";
import { date } from "./time.ts";

/** Where a link of each provider starts, and a sign-in. */
const starts: Record<SignInProvider, { link: string; signIn: string }> = {
  github: { link: "/auth/github/link", signIn: "/auth/github" },
  google: { link: "/auth/google/link", signIn: "/auth/google" },
};

/** The query that returns a link or a sign-in to the Account page. */
const back = new URLSearchParams({ return_to: accountPath }).toString();

/** What names `method` on the page: its provider, and a GitHub method's login. */
function nameOf(method: SignInMethod): string {
  const provider = providerNames[method.provider];
  return method.github_login ? `${provider} ${method.github_login}` : provider;
}

export interface AccountScreenProps {
  session: Session;
  /** The account's sign-in methods; none while they load. */
  methods?: readonly SignInMethod[];
  /** The providers the server has settings for. */
  providers: readonly SignInProvider[];
  /** The time the dates count from, in milliseconds since the epoch. */
  now: number;
  /** What failed, shown above the table. */
  failed?: string;
  /** The server asks for a newer sign-in to remove a method or delete the account: the page offers
   * to sign in again. */
  signInAgain?: "remove" | "delete";
  /** Confirmed Remove of `method`. */
  onRemove: (method: SignInMethod) => Promise<void>;
  /** The method whose Remove asks on the first render, as on the kit page. */
  defaultAsking?: SignInMethod;
  /** What a deletion of the account deletes and what blocks it; none while it loads. */
  deletion?: AccountDeletion;
  /** Confirmed Delete of the account. */
  onDelete: () => Promise<void>;
  /** Delete asks on the first render, as on the kit page. */
  defaultDeleting?: boolean;
}

/**
 * The Account page: the username with the GitHub login; the account's sign-in methods, each with
 * when it was linked and Remove while the account has two or more; a link for each provider the
 * server has, which returns here. Remove asks first and, for the method of this web session, says
 * that it signs this browser out.
 */
export function AccountScreen({
  session,
  methods,
  providers,
  now,
  failed,
  signInAgain,
  onRemove,
  defaultAsking,
  deletion,
  onDelete,
  defaultDeleting,
}: AccountScreenProps) {
  const [asking, setAsking] = useState(defaultAsking);
  const [removing, setRemoving] = useState(false);
  const removable = (methods?.length ?? 0) > 1;
  // A sign-in again with a provider the account has none of would reach another account.
  const again = providers.filter((provider) =>
    methods?.some((method) => method.provider === provider),
  );

  const columns: Column<SignInMethod>[] = [
    {
      key: "provider",
      label: "Method",
      width: "minmax(0, 2fr)",
      cell: (method) => (
        <TwoLine detail={method.github_login}>
          {providerNames[method.provider]}
        </TwoLine>
      ),
    },
    {
      key: "linked",
      label: "Linked",
      cell: (method) => date(method.linked_at, now),
    },
    {
      key: "actions",
      label: "Actions",
      actions: true,
      width: "96px",
      cell: (method) =>
        removable && (
          <Button
            kind="danger"
            size="small"
            aria-label={`Remove ${nameOf(method)}`}
            onClick={() => setAsking(method)}
          >
            Remove
          </Button>
        ),
    },
  ];

  return (
    <>
      <PageHeader
        title="Account"
        subtitle={
          <>
            {session.github_login
              ? `${session.username} · GitHub ${session.github_login}`
              : session.username}
            . Removing a method signs out its browsers; machines signed in with{" "}
            <code>skald</code> stay.
          </>
        }
        actions={providers.map((provider) => (
          <form
            key={provider}
            method="post"
            action={`${starts[provider].link}?${back}`}
          >
            <Button type="submit" size="large">
              Link {providerNames[provider]}
            </Button>
          </form>
        ))}
      />
      {signInAgain && (
        <Banner
          actions={again.map((provider) => ({
            label: `Sign in again with ${providerNames[provider]}`,
            onClick: () =>
              window.location.assign(`${starts[provider].signIn}?${back}`),
          }))}
        >
          To{" "}
          {signInAgain === "delete"
            ? "delete the account"
            : "remove a sign-in method"}
          , sign in again first: your last sign-in is too old.
        </Banner>
      )}
      {failed && <Banner>{failed}</Banner>}
      <Table
        label="Sign-in methods"
        columns={columns}
        rows={methods ?? []}
        rowKey={(method) => method.method}
        loading={!methods}
        loadingLabel="Loading sign-in methods"
        empty={<EmptyState>No sign-in methods.</EmptyState>}
      />
      {asking && (
        <ConfirmDialog
          title={`Remove ${nameOf(asking)}?`}
          confirm="Remove"
          busy={removing}
          onCancel={() => setAsking(undefined)}
          onConfirm={() => {
            setRemoving(true);
            onRemove(asking).finally(() => {
              setRemoving(false);
              setAsking(undefined);
            });
          }}
        >
          <p>
            It no longer signs in to your account, and the browsers signed in
            with it are signed out. Machines signed in with <code>skald</code>{" "}
            stay.
          </p>
          {asking.current && (
            <p>You signed in with it here: this signs this browser out too.</p>
          )}
        </ConfirmDialog>
      )}
      <DeletionPart
        username={session.username}
        deletion={deletion}
        onDelete={onDelete}
        defaultDeleting={defaultDeleting}
      />
    </>
  );
}

/** The command that hands this account's admin rights in `space` to a member, on its machine. */
const handOver = (space: DeletionSpace) =>
  `skald hand-over-admin ${space.name} <handle>`;

/**
 * The account's deletion: the spaces it deletes, those only the account is in; each space that
 * blocks it, where the account is the only admin and there are other members, with the command that
 * hands over admin rights; and, while none blocks, Delete. Delete asks first, says what goes at once
 * and what 30 days later, and confirms only once the typed username is `username`. Nothing shows
 * while `deletion` loads.
 */
function DeletionPart({
  username,
  deletion,
  onDelete,
  defaultDeleting = false,
}: {
  username: string;
  deletion?: AccountDeletion;
  onDelete: () => Promise<void>;
  defaultDeleting?: boolean;
}) {
  const heading = useId();
  const field = useId();
  const [asking, setAsking] = useState(defaultDeleting);
  const [typed, setTyped] = useState("");
  const [deleting, setDeleting] = useState(false);
  const close = useCallback(() => {
    setAsking(false);
    setTyped("");
  }, []);
  if (!deletion) return null;
  const { deletes, blocks } = deletion;

  return (
    <section aria-labelledby={heading} className={styles.deletion}>
      <h2 id={heading}>Delete account</h2>
      <p>
        {deletes.length > 0 ? (
          <>
            Deleting the account deletes the spaces only you are in:{" "}
            {deletes.map((space) => space.name).join(", ")}.
          </>
        ) : (
          "Deleting the account deletes no space: none has only you."
        )}{" "}
        The spaces you share stay with their members.
      </p>
      {blocks.length > 0 ? (
        <>
          <Banner>
            You are the only admin of{" "}
            {blocks.length === 1 ? "a space" : "spaces"} with other members.
            Before deleting the account, make a member the admin on your
            machine:
          </Banner>
          <ul className={styles.blocks}>
            {blocks.map((space) => (
              <li key={space.space}>
                {space.name}: <code>{handOver(space)}</code>
              </li>
            ))}
          </ul>
        </>
      ) : (
        <div>
          <Button kind="danger" onClick={() => setAsking(true)}>
            Delete account
          </Button>
        </div>
      )}
      {asking && (
        <ConfirmDialog
          title="Delete your account?"
          confirm="Delete account"
          busy={deleting}
          confirmDisabled={typed !== username}
          onCancel={close}
          onConfirm={() => {
            setDeleting(true);
            onDelete().finally(() => {
              setDeleting(false);
              close();
            });
          }}
        >
          <p>
            At once, every browser and machine signed in to it is signed out,
            its sign-in methods are removed and its username is free; the spaces
            only you are in are deleted with their history. Nothing undoes it.
          </p>
          <p>
            30 days later the server erases the rest of its data, but for what
            the spaces you shared keep: your messages and your machines&apos;
            certificates.
          </p>
          <label htmlFor={field} className={styles.confirm}>
            Type your username, <code>{username}</code>, to confirm
          </label>
          <input
            id={field}
            className={styles.field}
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
          />
        </ConfirmDialog>
      )}
    </section>
  );
}

/** The problem type of a refusal that asks for a newer sign-in. */
const signInTooOld = "https://skald.so/problems/sign-in-too-old";

/**
 * The Account page of the session's account with its methods, the server's providers and what a
 * deletion deletes and blocks from the API. A removal calls `onChanged`, which loads the session
 * again: the page lists the methods again once the new session comes, or the sign-in page shows
 * once it has ended. One load at a time, so none is aborted in flight. A deletion calls
 * `onDeleted`; a refusal as blocked shows the spaces that block it. When the API refuses the
 * session, `onSignedOut` shows the sign-in page.
 */
export function Account({
  session,
  onChanged,
  onDeleted,
  onSignedOut,
}: {
  session: Session;
  onChanged: () => void;
  onDeleted: () => void;
  onSignedOut: () => void;
}) {
  const [listed, setListed] = useState<{
    methods: SignInMethod[];
    at: number;
  }>();
  const [providers, setProviders] = useState<SignInProvider[]>([]);
  const [deletion, setDeletion] = useState<AccountDeletion>();
  const [failed, setFailed] = useState<string>();
  const [signInAgain, setSignInAgain] = useState<"remove" | "delete">();

  useEffect(() => {
    const request = new AbortController();
    const options = { headers: versioned, signal: request.signal };
    Promise.all([listSignInMethods(options), getSignInProviders(options)]).then(
      ([methods, offered]) => {
        if (methods.status === 401) return onSignedOut();
        if (methods.status !== 200 || offered.status !== 200)
          return setFailed("The sign-in methods did not load.");
        setListed({ methods: methods.data.methods, at: Date.now() });
        setProviders(offered.data.providers);
      },
      () => {
        if (!request.signal.aborted)
          setFailed("The sign-in methods did not load.");
      },
    );
    getAccountDeletion(options)
      .then((answer) => signedIn(answer, "GET /account/deletion"))
      .then(
        (loaded) => (loaded ? setDeletion(loaded) : onSignedOut()),
        () => {
          if (!request.signal.aborted)
            setFailed("What a deletion deletes did not load.");
        },
      );
    return () => request.abort();
    // A new `session` after a removal lists the methods again.
  }, [session, onSignedOut]);

  async function remove(method: SignInMethod) {
    setFailed(undefined);
    try {
      const removed = await removeSignInMethod(method.method, {
        headers: versioned,
      });
      if (removed.status === 401) return onSignedOut();
      if (removed.status === 403 && removed.data.type === signInTooOld)
        return setSignInAgain("remove");
      if (removed.status !== 204)
        setFailed("Removing the sign-in method failed.");
    } catch {
      setFailed("Removing the sign-in method failed.");
    }
    onChanged();
  }

  async function deleteIt() {
    setFailed(undefined);
    try {
      const deleted = await deleteAccount({ headers: versioned });
      if (deleted.status === 204) return onDeleted();
      if (deleted.status === 401) return onSignedOut();
      if (deleted.status === 403 && deleted.data.type === signInTooOld)
        return setSignInAgain("delete");
      if (deleted.status === 409) {
        const blocks = deleted.data.spaces;
        return setDeletion((shown) => ({
          deletes: (shown?.deletes ?? []).filter(
            (space) => !blocks.some((block) => block.space === space.space),
          ),
          blocks,
        }));
      }
      setFailed("Deleting the account failed.");
    } catch {
      setFailed("Deleting the account failed.");
    }
  }

  return (
    <AccountScreen
      session={session}
      methods={listed?.methods}
      providers={providers}
      now={listed?.at ?? 0}
      failed={failed}
      signInAgain={signInAgain}
      onRemove={remove}
      deletion={deletion}
      onDelete={deleteIt}
    />
  );
}
