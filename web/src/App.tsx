import { useEffect, useState } from "react";
import { getHealth } from "./api/skald.ts";

/** The server health: `ok` and `unavailable` as in the spec; `unreachable` when there is no valid answer. */
type Health = "checking" | "ok" | "unavailable" | "unreachable";

export function App() {
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
    <main>
      <h1>Skald</h1>
      <p role="status">
        Server: {health === "checking" ? "checking…" : health}
      </p>
    </main>
  );
}
