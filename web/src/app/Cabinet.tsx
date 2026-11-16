import { useState, type ReactNode } from "react";
import { useLocation, useNavigate, type NavigateFunction } from "react-router";
import { endSession } from "../api/skald.ts";
import type { ListedSpace, Session } from "../api/skald.schemas.ts";
import type { DropdownOption } from "../ui/Dropdown.tsx";
import { Shell, type ShellAccount, type ShellSection } from "../ui/Shell.tsx";
import { accountPath, providerNames } from "./names.ts";
import { versioned } from "./api.ts";

/** The cabinet's sections, each on its page. */
const sections: ShellSection[] = [
  { key: "access", label: "Access", href: "/access" },
];

/**
 * The cabinet shell around `children`, the page area, for the account of `session` with `spaces`:
 * the space switcher with the personal space marked, or, with no space, what makes one; the side
 * menu's sections, followed in the page; and the account footer, whose Sign out ends this web
 * session and then calls `onSignedOut`. `menuOpen` opens the side menu at phone width, as on the
 * kit page.
 */
export function Cabinet({
  session,
  spaces,
  onSignedOut,
  menuOpen = false,
  children,
}: {
  session: Session;
  spaces: readonly ListedSpace[];
  onSignedOut: () => void;
  menuOpen?: boolean;
  children: ReactNode;
}) {
  const location = useLocation();
  const navigate = useNavigate();
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
      sections={sections}
      current={
        sections.find((section) => section.href === location.pathname)?.key ??
        ""
      }
      onFollow={(href) => navigate(href)}
      spaces={options}
      noSpaces={
        <p>
          No space yet. Sign in a machine with <code>skald</code> to make your
          personal space.
        </p>
      }
      space={space}
      onSpaceChange={setSpace}
      account={account(session, navigate, onSignedOut)}
      defaultMenuOpen={menuOpen}
    >
      {children}
    </Shell>
  );
}

/**
 * The account footer of `session`: the username; the method it signed in with, and the GitHub login
 * of the account's GitHub method, as "GitHub · ana" or "Google · GitHub ana"; its menu's Account
 * opens the Account page, and Sign out calls `onSignedOut` once the server has answered.
 */
function account(
  session: Session,
  navigate: NavigateFunction,
  onSignedOut: () => void,
): ShellAccount {
  const method =
    session.sign_in_method && providerNames[session.sign_in_method];
  const login = session.github_login;
  const signIn = [
    method,
    login && (method === providerNames.github ? login : `GitHub ${login}`),
  ].filter(Boolean);
  return {
    name: session.username,
    initials: initials(session.username),
    signIn: signIn.join(" · "),
    actions: [
      { label: "Account", onSelect: () => navigate(accountPath) },
      {
        label: "Sign out",
        onSelect: () => {
          // Whatever it answers, the web app loads again: the sign-in page once the session ended.
          endSession({ headers: versioned }).then(onSignedOut, onSignedOut);
        },
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
