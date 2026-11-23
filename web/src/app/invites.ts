import { useEffect, useState } from "react";
import { listSpaceInvites, revokeSpaceInvite } from "../api/skald.ts";
import type { ListedInvite, ListedSpace } from "../api/skald.schemas.ts";
import { versioned } from "./api.ts";

export interface SpaceInvite {
  space: ListedSpace;
  invite: ListedInvite;
}

export interface InviteLinksData {
  /** Undefined while loading or when the account administers none of the spaces. */
  rows?: readonly SpaceInvite[];
  now: number;
  failed?: string;
  revoke: (row: SpaceInvite) => Promise<void>;
}

/** The active links from the spaces the account administers, refreshed after revocation. */
export function useInviteLinks(
  spaces: readonly ListedSpace[],
  onSignedOut: () => void,
): InviteLinksData {
  const [loaded, setLoaded] = useState<{
    spaces: readonly ListedSpace[];
    rows?: SpaceInvite[];
    now: number;
  }>();
  const [error, setError] = useState<{
    spaces: readonly ListedSpace[];
    text: string;
  }>();
  const [changes, setChanges] = useState(0);

  useEffect(() => {
    const request = new AbortController();
    async function load() {
      try {
        const listed = await Promise.all(
          spaces.map((space) =>
            listSpaceInvites(space.space, {
              headers: versioned,
              signal: request.signal,
            }),
          ),
        );
        if (request.signal.aborted) return;
        if (listed.some(({ status }) => status === 401)) return onSignedOut();

        const rows: SpaceInvite[] = [];
        let admin = false;
        for (const [index, answer] of listed.entries()) {
          if (answer.status === 403 || answer.status === 404) continue;
          if (answer.status !== 200)
            throw new Error(`List invites: ${answer.status}`);
          admin = true;
          for (const invite of answer.data.invites) {
            rows.push({ space: spaces[index]!, invite });
          }
        }
        setLoaded({ spaces, rows: admin ? rows : undefined, now: Date.now() });
      } catch {
        if (!request.signal.aborted) {
          setError({ spaces, text: "The invite links did not load." });
        }
      }
    }
    void load();
    return () => request.abort();
  }, [changes, spaces, onSignedOut]);

  async function revoke({ space, invite }: SpaceInvite) {
    setError(undefined);
    try {
      const { status } = await revokeSpaceInvite(space.space, invite.invite, {
        headers: versioned,
      });
      if (status === 401) return onSignedOut();
      if (status !== 204 && status !== 404)
        throw new Error(`Revoke invite: ${status}`);
      setLoaded((current) =>
        current?.spaces === spaces
          ? {
              ...current,
              rows: current.rows?.filter(
                (row) => row.invite.invite !== invite.invite,
              ),
            }
          : current,
      );
    } catch {
      setError({ spaces, text: "The invite link could not be revoked." });
    } finally {
      setChanges((value) => value + 1);
    }
  }

  return {
    rows: loaded?.spaces === spaces ? loaded.rows : undefined,
    now: loaded?.now ?? 0,
    failed: error?.spaces === spaces ? error.text : undefined,
    revoke,
  };
}
