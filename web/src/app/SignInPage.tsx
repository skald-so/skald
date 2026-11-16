import { useId, type ReactNode } from "react";
import type { SignInProvider, SignInProviders } from "../api/skald.schemas.ts";
import a11y from "../ui/a11y.module.css";
import { Button } from "../ui/Button.tsx";
import { GitHubIcon, GoogleIcon, LogoIcon } from "../ui/icons.tsx";
import styles from "./SignInPage.module.css";

/** Each provider's button: its sign-in's start and its face, as 03-sign-in.png. */
const faces: Record<
  SignInProvider,
  { start: string; name: string; icon: ReactNode; look?: string }
> = {
  github: {
    start: "/auth/github",
    name: "GitHub",
    icon: <GitHubIcon />,
    look: styles.github,
  },
  google: { start: "/auth/google", name: "Google", icon: <GoogleIcon /> },
};

/**
 * The sign-in page of 03-sign-in.png: a button for each provider the server has, which signs in
 * and returns to `returnTo`, a page of the web app; the note on sign-up; and the terminal box, which
 * opens the device flow's verification page with the code typed in it.
 */
export function SignInPage({
  providers,
  returnTo,
}: {
  providers: SignInProviders;
  returnTo: string;
}) {
  const heading = useId();
  const terminal = useId();
  const field = useId();
  const back = new URLSearchParams({ return_to: returnTo }).toString();

  return (
    <CenteredPage>
      <section aria-labelledby={heading} className={styles.card}>
        <h1 id={heading}>Sign in</h1>
        {providers.providers.map((provider) => (
          <a
            key={provider}
            href={`${faces[provider].start}?${back}`}
            className={[styles.provider, faces[provider].look]
              .filter(Boolean)
              .join(" ")}
          >
            {faces[provider].icon}
            Continue with {faces[provider].name}
          </a>
        ))}
        {providers.providers.length === 0 ? (
          <p className={styles.note}>This server has no sign-in set up.</p>
        ) : (
          <p className={styles.note}>
            {providers.sign_up === "open"
              ? "New here? Signing in creates an account."
              : "New here? Signing in creates an account only for the people this server lists."}
          </p>
        )}
      </section>
      <section aria-labelledby={terminal} className={styles.card}>
        <h2 id={terminal}>Signing in from a terminal?</h2>
        <p className={styles.hint}>
          Enter the code that <code>skald</code> shows.
        </p>
        <form method="get" action="/device" className={styles.code}>
          <label htmlFor={field} className={a11y.visuallyHidden}>
            Code
          </label>
          <input
            id={field}
            name="user_code"
            required
            placeholder="XXXX-XXXX"
            autoComplete="off"
            autoCapitalize="characters"
            spellCheck={false}
          />
          <Button type="submit">Continue</Button>
        </form>
      </section>
    </CenteredPage>
  );
}

/** A page of the web app outside the cabinet: the logo above its cards, in the middle. */
export function CenteredPage({ children }: { children: ReactNode }) {
  return (
    <div className={styles.page}>
      <main className={styles.main}>
        <span className={styles.logo}>
          <LogoIcon size={30} />
          skald
        </span>
        {children}
      </main>
    </div>
  );
}
