import { useCallback, useEffect, useState } from "react";
import { BrowserRouter, Link, Route, Routes } from "react-router";
import {
  getHealth,
  getSession,
  getSignInProviders,
  getSpaces,
} from "./api/skald.ts";
import type {
  ListedSpace,
  Session,
  SignInProviders,
} from "./api/skald.schemas.ts";
import { Access } from "./app/Access.tsx";
import { Account } from "./app/Account.tsx";
import { versioned } from "./app/api.ts";
import { Cabinet } from "./app/Cabinet.tsx";
import { Machines } from "./app/Machines.tsx";
import { Members } from "./app/Members.tsx";
import { accountPath } from "./app/names.ts";
import { CenteredPage, SignInPage } from "./app/SignInPage.tsx";
import { Usage } from "./app/Usage.tsx";
import styles from "./app/SignInPage.module.css";
import { Button } from "./ui/Button.tsx";
import { EmptyState } from "./ui/EmptyState.tsx";
import { Loading } from "./ui/Loading.tsx";
import { PageHeader } from "./ui/PageHeader.tsx";

/** What the web app shows: the cabinet with a web session, else the sign-in page. */
type View =
  | { kind: "loading" }
  | { kind: "failed" }
  | { kind: "signed-out"; providers: SignInProviders }
  | { kind: "signed-in"; session: Session; spaces: ListedSpace[] };

/**
 * The web app. With a web session, the cabinet of its account on every page; without one, or when
 * the API refuses it, the sign-in page, which returns to the page opened.
 */
export function App() {
  const [view, setView] = useState<View>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);
  /** Loads the web app again, as when it starts. */
  const reload = useCallback(() => {
    setView({ kind: "loading" });
    setAttempt((attempt) => attempt + 1);
  }, []);
  /** Loads the session again behind the page shown: the sign-in page once it has ended. */
  const refresh = useCallback(() => setAttempt((attempt) => attempt + 1), []);

  useEffect(() => {
    const request = new AbortController();
    load(request.signal).then(setView, () => {
      if (!request.signal.aborted) setView({ kind: "failed" });
    });
    return () => request.abort();
  }, [attempt]);

  switch (view.kind) {
    case "loading":
      return (
        <CenteredPage>
          <Loading />
        </CenteredPage>
      );
    case "failed":
      return <Failed onRetry={reload} />;
    case "signed-out":
      return (
        <SignInPage
          providers={view.providers}
          returnTo={
            window.location.pathname +
            window.location.search +
            window.location.hash
          }
        />
      );
    case "signed-in":
      return (
        <BrowserRouter>
          <Cabinet
            session={view.session}
            spaces={view.spaces}
            onSignedOut={reload}
          >
            <Routes>
              <Route
                path="/"
                element={
                  <Machines
                    username={view.session.username}
                    onSignedOut={reload}
                  />
                }
              />
              <Route
                path="/members"
                element={<Members onSignedOut={reload} />}
              />
              <Route path="/access" element={<Access onSignedOut={reload} />} />
              <Route path="/usage" element={<Usage onSignedOut={reload} />} />
              <Route
                path={accountPath}
                element={
                  <Account
                    session={view.session}
                    onChanged={refresh}
                    onSignedOut={reload}
                  />
                }
              />
              <Route path="*" element={<NotFound />} />
            </Routes>
          </Cabinet>
        </BrowserRouter>
      );
  }
}

/**
 * The account's session and spaces; without a session, or when the API refuses it, the sign-in
 * providers. Throws when the server gives neither.
 */
async function load(signal: AbortSignal): Promise<View> {
  const [session, spaces] = await Promise.all([
    getSession({ headers: versioned, signal }),
    getSpaces({ headers: versioned, signal }),
  ]);
  if (session.status === 200 && spaces.status === 200)
    return {
      kind: "signed-in",
      session: session.data,
      spaces: spaces.data.spaces,
    };
  if (session.status !== 401 && spaces.status !== 401)
    throw new Error(`GET /session ${session.status}, /spaces ${spaces.status}`);
  const providers = await getSignInProviders({ headers: versioned, signal });
  if (providers.status !== 200)
    throw new Error(`GET /auth/providers ${providers.status}`);
  return { kind: "signed-out", providers: providers.data };
}

/** The server health: `ok` and `unavailable` as in the spec; `unreachable` when there is no valid answer. */
type Health = "checking" | "ok" | "unavailable" | "unreachable";

/** What the web app shows when it could not load: the server health, and a new attempt. */
function Failed({ onRetry }: { onRetry: () => void }) {
  const [health, setHealth] = useState<Health>("checking");

  useEffect(() => {
    const request = new AbortController();
    getHealth({ signal: request.signal })
      .then((response) => {
        if (response.status === 200) setHealth("ok");
        else if (response.status === 503) setHealth("unavailable");
        else setHealth("unreachable");
      })
      .catch(() => {
        if (!request.signal.aborted) setHealth("unreachable");
      });
    return () => request.abort();
  }, []);

  return (
    <CenteredPage>
      <section className={styles.card} aria-labelledby="failed">
        <h1 id="failed">Skald did not load</h1>
        <p role="status" className={styles.note}>
          Server: {health === "checking" ? "checking…" : health}
        </p>
        <Button onClick={onRetry}>Try again</Button>
      </section>
    </CenteredPage>
  );
}

function NotFound() {
  return (
    <>
      <PageHeader title="Not found" />
      <EmptyState action={<Link to="/">Open the cabinet</Link>}>
        The web app has no such page.
      </EmptyState>
    </>
  );
}
