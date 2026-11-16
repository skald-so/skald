import { useEffect, useState } from "react";
import {
  getSignInProviders,
  listSignInMethods,
  removeSignInMethod,
} from "../api/skald.ts";
import type {
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
import { versioned } from "./api.ts";
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
  /** The server asks for a newer sign-in: the page offers to sign in again. */
  signInAgain?: boolean;
  /** Confirmed Remove of `method`. */
  onRemove: (method: SignInMethod) => Promise<void>;
  /** The method whose Remove asks on the first render, as on the kit page. */
  defaultAsking?: SignInMethod;
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
  signInAgain = false,
  onRemove,
  defaultAsking,
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
          To remove a sign-in method, sign in again first: your last sign-in is
          too old.
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
          {asking.provider === session.sign_in_method && (
            <p>You signed in with it here: this signs this browser out too.</p>
          )}
        </ConfirmDialog>
      )}
    </>
  );
}

/** The problem type of a refusal that asks for a newer sign-in. */
const signInTooOld = "https://skald.so/problems/sign-in-too-old";

/**
 * The Account page of the session's account with its methods and the server's providers from the
 * API, listed again after each removal. A removal calls `onChanged`, which loads the session again;
 * when the API refuses the session, `onSignedOut` shows the sign-in page.
 */
export function Account({
  session,
  onChanged,
  onSignedOut,
}: {
  session: Session;
  onChanged: () => void;
  onSignedOut: () => void;
}) {
  const [listed, setListed] = useState<{
    methods: SignInMethod[];
    at: number;
  }>();
  const [providers, setProviders] = useState<SignInProvider[]>([]);
  const [failed, setFailed] = useState<string>();
  const [signInAgain, setSignInAgain] = useState(false);
  /** Counts the removals, each of which lists the methods again. */
  const [changes, setChanges] = useState(0);

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
    return () => request.abort();
  }, [changes, onSignedOut]);

  async function remove(method: SignInMethod) {
    setFailed(undefined);
    try {
      const removed = await removeSignInMethod(method.method, {
        headers: versioned,
      });
      if (removed.status === 401) return onSignedOut();
      if (removed.status === 403 && removed.data.type === signInTooOld)
        return setSignInAgain(true);
      if (removed.status !== 204)
        setFailed("Removing the sign-in method failed.");
    } catch {
      setFailed("Removing the sign-in method failed.");
    }
    setChanges((changes) => changes + 1);
    onChanged();
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
    />
  );
}
