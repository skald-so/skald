import type { Session } from "../api/skald.schemas.ts";
import { PageHeader } from "../ui/PageHeader.tsx";

/** The cabinet's first page: it names the account. */
export function Home({ session }: { session: Session }) {
  return (
    <PageHeader
      title={session.username}
      subtitle="Your account on this server."
    />
  );
}
