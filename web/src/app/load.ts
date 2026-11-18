// How a section of the web app loads what it shows.
import { useEffect, useState } from "react";

/**
 * What `load` gives for `key`, loaded again whenever `key` changes, or whether it failed; nothing
 * while it loads or without a key. When `load` gives `null`, as when the API refuses the session,
 * `onSignedOut` shows the sign-in page. An answer for a key before is dropped.
 */
export function useLoaded<T>(
  key: string | undefined,
  load: (key: string) => Promise<T | null>,
  onSignedOut: () => void,
): { value?: T; failed?: true } | undefined {
  const [loaded, setLoaded] = useState<{
    key: string;
    value?: T;
    failed?: true;
  }>();
  useEffect(() => {
    if (key === undefined) return;
    // Not aborted when the key changes or another page opens: the answer comes whole.
    let current = true;
    const settle = (value?: T, failed?: true) => {
      if (current) setLoaded({ key, value, failed });
    };
    load(key).then(
      (value) => (value ? settle(value) : current && onSignedOut()),
      () => settle(undefined, true),
    );
    return () => {
      current = false;
    };
  }, [key, load, onSignedOut]);
  return loaded?.key === key ? loaded : undefined;
}
