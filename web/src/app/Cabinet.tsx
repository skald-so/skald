import { useState, type ReactNode } from "react";
import type { ListedSpace, Session } from "../api/skald.schemas.ts";
import type { DropdownOption } from "../ui/Dropdown.tsx";
import { Shell, type ShellAccount } from "../ui/Shell.tsx";

/** The providers' names for people. */
const methods = { github: "GitHub", google: "Google" } as const;

/** Where the account's sign-in methods are: the server's page of them. */
const signInMethodsPage = "/sign-in-methods";

/**
 * The cabinet shell around `children`, the page area, for the account of `session` with `spaces`:
 * the space switcher with the personal space marked, or, with no space, what makes one; and the
 * account footer. `menuOpen` opens the side menu at phone width, as on the kit page.
 */
export function Cabinet({
  session,
  spaces,
  menuOpen = false,
  children,
}: {
  session: Session;
  spaces: readonly ListedSpace[];
  menuOpen?: boolean;
  children: ReactNode;
}) {
  const [space, setSpace] = useState(
    () => (spaces.find((space) => space.personal) ?? spaces[0])?.space ?? "",
  );
  const options: DropdownOption<string>[] = spaces.map((space) => ({
    value: space.space,
    label: space.name,
    mark: space.personal ? "personal" : undefined,
  }));

  return (
    <Shell
      sections={[]}
      current=""
      spaces={options}
      noSpaces={
        <p>
          No space yet. Sign in a machine with <code>skald</code> to make your
          personal space.
        </p>
      }
      space={space}
      onSpaceChange={setSpace}
      account={account(session)}
      defaultMenuOpen={menuOpen}
    >
      {children}
    </Shell>
  );
}

/**
 * The account footer of `session`: the username; the method it signed in with, and the GitHub login
 * of the account's GitHub method, as "GitHub · ana" or "Google · GitHub ana".
 */
function account(session: Session): ShellAccount {
  const method = session.sign_in_method && methods[session.sign_in_method];
  const login = session.github_login;
  const signIn = [
    method,
    login && (method === methods.github ? login : `GitHub ${login}`),
  ].filter(Boolean);
  return {
    name: session.username,
    initials: initials(session.username),
    signIn: signIn.join(" · "),
    actions: [
      {
        label: "Sign-in methods",
        onSelect: () => window.location.assign(signInMethodsPage),
      },
    ],
  };
}

/** The initials of `username`: of its first two words, as "AK" of `ana-kovac`, or "A" of `ana`. */
function initials(username: string): string {
  return username
    .split(/[-.]/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]!.toUpperCase())
    .join("");
}
