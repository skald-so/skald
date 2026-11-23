import { CenteredPage } from "./SignInPage.tsx";
import styles from "./SignInPage.module.css";

/** Instructions for opening an invite link in the CLI. */
export function InvitePage() {
  return (
    <CenteredPage>
      <section className={styles.card} aria-labelledby="invite-heading">
        <h1 id="invite-heading">Accept an invite</h1>
        <p className={styles.hint}>
          Run <code>skald accept-invite</code> in a terminal. Paste the full
          link when prompted.
        </p>
        <p className={styles.hint}>
          If sign-up is closed, <code>skald</code> can create your account with
          this link.
        </p>
      </section>
    </CenteredPage>
  );
}
