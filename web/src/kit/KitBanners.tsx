import { useEffect, useState } from "react";
import { Banner } from "../ui/Banner.tsx";
import { Button } from "../ui/Button.tsx";
import { LoopIcon } from "../ui/icons.tsx";
import { busyMs } from "./KitButtons.tsx";
import styles from "./KitPage.module.css";

export function KitBanners() {
  const [pausing, setPausing] = useState(false);
  const [shown, setShown] = useState(false);

  useEffect(() => {
    if (!pausing) return;
    const timer = setTimeout(() => setPausing(false), busyMs);
    return () => clearTimeout(timer);
  }, [pausing]);

  return (
    <section aria-labelledby="kit-banners" className={styles.section}>
      <h2 id="kit-banners">Warning banner</h2>
      <div className={styles.banners}>
        <span className={styles.caption}>With an icon and an action</span>
        <Banner
          icon={<LoopIcon />}
          actions={[
            {
              label: "Pause thread",
              busy: pausing,
              busyLabel: "Pausing the thread",
              onClick: () => setPausing(true),
            },
          ]}
        >
          <strong>14 messages between agents</strong> in this thread without a
          human.
        </Banner>
        <span className={styles.caption}>With an action</span>
        <Banner actions={[{ label: "Make Lee an admin" }]}>
          <strong>You are the only admin.</strong> If you lose all your devices
          and the recovery key, nobody can manage this space.
        </Banner>
        <span className={styles.caption}>Without actions</span>
        <Banner>
          <strong>ana-desktop is not verified.</strong> Compare its key before
          you trust its messages.
        </Banner>
        <span className={styles.caption}>Appears on demand</span>
        <div>
          <Button onClick={() => setShown(!shown)}>
            {shown ? "Hide the warning" : "Show a warning"}
          </Button>
        </div>
        {shown && (
          <Banner actions={[{ label: "Extend the lease" }]}>
            <strong>The lease of review-42 runs out in 1 minute.</strong>
          </Banner>
        )}
      </div>
    </section>
  );
}
