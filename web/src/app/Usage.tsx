import { useContext } from "react";
import { getAccountUsage, getSpaceUsage } from "../api/skald.ts";
import type { AccountUsage, SpaceUsage } from "../api/skald.schemas.ts";
import { Banner } from "../ui/Banner.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { Loading } from "../ui/Loading.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import { UsageCard } from "../ui/UsageCard.tsx";
import { signedIn, versioned } from "./api.ts";
import { useLoaded } from "./load.ts";
import { NoSpace } from "./NoSpace.tsx";
import { ChosenSpace } from "./space.ts";
import { period, size } from "./units.ts";
import styles from "./Usage.module.css";

export interface UsageScreenProps {
  /** The name of the space the switcher chose; none while the account has no space. */
  space?: string;
  /** The account's owned spaces and devices; none while they load. */
  account?: AccountUsage;
  /** The chosen space's messages, service agents and limits; none while they load. */
  usage?: SpaceUsage;
  /** What failed, shown above the cards. */
  failed?: string;
}

/** The account's usage and the chosen space's usage with settings limits or plan allowances. */
export function UsageScreen({
  space,
  account,
  usage,
  failed,
}: UsageScreenProps) {
  const loading =
    !failed && (!account || (space !== undefined && usage === undefined));
  const planAllowance = usage?.plan !== undefined;
  return (
    <>
      <PageHeader
        title="Usage"
        subtitle={
          space === undefined ? (
            "What your account uses, counted from metadata only."
          ) : (
            <>What your account and {space} use, counted from metadata only.</>
          )
        }
      />
      {failed && <Banner>{failed}</Banner>}
      {space === undefined && (
        <EmptyState>
          <NoSpace />
        </EmptyState>
      )}
      {loading ? (
        <Loading label="Loading usage" />
      ) : (
        <>
          <section aria-label="Limits" className={styles.cards}>
            {account && (
              <>
                <UsageCard
                  label={
                    account.owned_spaces.free_only ? "Free spaces" : "Spaces"
                  }
                  count={account.owned_spaces.count}
                  limit={account.owned_spaces.limit}
                />
                <UsageCard label="Devices" {...account.devices} />
              </>
            )}
            {usage && (
              <>
                <UsageCard
                  label="Messages this month"
                  {...usage.monthly_messages}
                  planAllowance={planAllowance}
                />
                <UsageCard label="Service agents" {...usage.service_agents} />
              </>
            )}
          </section>
          {account && <OwnedSpaces owned={account.owned_spaces} />}
          {usage && <Messages usage={usage} />}
        </>
      )}
    </>
  );
}

/** The spaces the account owns by name, and that its personal space counts before it is made. */
function OwnedSpaces({ owned }: { owned: AccountUsage["owned_spaces"] }) {
  return (
    <section aria-labelledby="usage-owned" className={styles.section}>
      <h2 id="usage-owned">Spaces you own</h2>
      {owned.spaces.length > 0 && (
        <ul className={styles.names}>
          {owned.spaces.map((space) => (
            <li key={space.space}>{space.name}</li>
          ))}
        </ul>
      )}
      {owned.free_only ? (
        <p>A Free personal space counts before it is made.</p>
      ) : (
        owned.count > owned.spaces.length && (
          <p>Your personal space counts before it is made.</p>
        )
      )}
    </section>
  );
}

/** The chosen space's message size and retention limits or plan allowances. */
function Messages({ usage }: { usage: SpaceUsage }) {
  const largest = usage.message_size.limit;
  const kept = usage.mailbox_retention.limit;
  const history = usage.history_retention.limit;
  return (
    <section aria-labelledby="usage-messages" className={styles.section}>
      <h2 id="usage-messages">Messages</h2>
      {usage.plan === undefined ? (
        <p>
          {largest === undefined
            ? "A message's size has no limit."
            : `A message holds up to ${size(largest)}.`}{" "}
          {kept === undefined
            ? "Undelivered messages are kept until delivered, with no limit."
            : `Undelivered messages are kept for ${period(kept)}.`}{" "}
          {history === undefined
            ? "Channel history is kept for good."
            : `Channel history is kept for ${period(history)}.`}
        </p>
      ) : (
        <p>
          Plan allowances include{" "}
          {largest === undefined
            ? "unlimited message size"
            : `${size(largest)} per message`}
          ,{" "}
          {kept === undefined
            ? "unlimited retention for undelivered messages"
            : `${period(kept)} for undelivered messages`}
          , and{" "}
          {history === undefined
            ? "unlimited channel history"
            : `${period(history)} of channel history`}
          . Server settings govern enforcement.
        </p>
      )}
    </section>
  );
}

/** The usage of the account, or `null` when the API refuses the session; throws when it gives neither. */
async function fetchAccount(): Promise<AccountUsage | null> {
  return signedIn(
    await getAccountUsage({ headers: versioned }),
    "GET /account/usage",
  );
}

/** The usage of `space`, or `null` when the API refuses the session; throws when it gives neither. */
async function fetchSpace(space: string): Promise<SpaceUsage | null> {
  return signedIn(
    await getSpaceUsage(space, { headers: versioned }),
    `GET /spaces/${space}/usage`,
  );
}

/**
 * The Usage section of the account and of the space the switcher chose, from the API. When the API
 * refuses the session, `onSignedOut` shows the sign-in page.
 */
export function Usage({ onSignedOut }: { onSignedOut: () => void }) {
  const space = useContext(ChosenSpace);
  const account = useLoaded("account", fetchAccount, onSignedOut);
  const usage = useLoaded(space?.space, fetchSpace, onSignedOut);
  return (
    <UsageScreen
      space={space?.name}
      account={account?.value}
      usage={usage?.value}
      failed={
        account?.failed || usage?.failed ? "The usage did not load." : undefined
      }
    />
  );
}
