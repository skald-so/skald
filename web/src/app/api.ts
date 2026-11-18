// What every request of the web app sends with the generated client, and what its answers give.

/**
 * The header of the protocol version the web app speaks, for every request of the generated client
 * (docs/protocol/README.md, "Stating the version").
 */
export const versioned = { "Skald-Protocol-Version": "1" };

/**
 * The data of `answer`, the generated client's answer to `request`, as `GET /account/devices`, or
 * `null` when the API refuses the session; throws when it gives neither.
 */
export function signedIn<A extends { status: number; data: unknown }>(
  answer: A,
  request: string,
): Extract<A, { status: 200 }>["data"] | null {
  if (answer.status === 401) return null;
  if (answer.status !== 200) throw new Error(`${request} ${answer.status}`);
  return answer.data as Extract<A, { status: 200 }>["data"];
}
