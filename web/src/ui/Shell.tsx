import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import a11y from "./a11y.module.css";
import { Dropdown, type DropdownOption } from "./Dropdown.tsx";
import { CrossIcon, LogoIcon, MenuIcon, MoreIcon } from "./icons.tsx";
import { Menu, type MenuAction } from "./Menu.tsx";
import styles from "./Shell.module.css";
import { CountBadge, type Tone } from "./Status.tsx";
import tones from "./Status.module.css";

export interface ShellBadge {
  count: number;
  /** Follows the number for a screen reader only, as "unread" in "2 unread". */
  label: string;
  tone: Tone;
}

export interface ShellLink {
  /** Names the link in `current`. */
  key: string;
  label: string;
  href: string;
  badge?: ShellBadge;
}

export interface ShellSection extends ShellLink {
  /** Sub-items under the section, as the channels under Messages. */
  items?: readonly ShellLink[];
}

export interface ShellAccount {
  name: string;
  /** In the avatar, as "AK". */
  initials: string;
  /** The sign-in method and the login, as "GitHub · ana". */
  signIn: string;
  actions: readonly MenuAction[];
}

export interface ShellProps {
  sections: readonly ShellSection[];
  /** The key of the current section or sub-item; a section with the current sub-item is marked too. */
  current: string;
  /** A click on a section or a sub-item, before the browser follows its link. */
  onNavigate?: (key: string) => void;
  spaces: readonly DropdownOption<string>[];
  space: string;
  /** A choice of another space. */
  onSpaceChange: (space: string) => void;
  account: ShellAccount;
  /** The link of the logo. */
  home?: string;
  /** Opens the side menu at phone width on the first render, as on the kit page. */
  defaultMenuOpen?: boolean;
  /** The page area: a `PageHeader` and the screen. */
  children: ReactNode;
}

/** Phone width: the side menu hides behind the menu button. The same width as in Shell.module.css. */
const phoneQuery = "(width < 800px)";

/**
 * The frame of every cabinet screen, as 08-machines.png: the side menu with the logo, the space
 * switcher, the sections and the account footer, and the page area.
 *
 * At phone width the side menu hides behind a menu button and opens over the page; choosing a
 * section, Escape or a click on the page closes it, and the page is out of reach while it is open.
 */
export function Shell({
  sections,
  current,
  onNavigate,
  spaces,
  space,
  onSpaceChange,
  account,
  home = "/",
  defaultMenuOpen = false,
  children,
}: ShellProps) {
  const panelId = useId();
  const menuButton = useRef<HTMLButtonElement>(null);
  const phone = usePhoneWidth();
  const [open, setOpen] = useState(defaultMenuOpen);
  const menuOpen = phone && open;

  /** Closes the side menu; `refocus` returns the focus to the menu button. */
  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) menuButton.current!.focus();
  }

  // The side menu is always shown above phone width: it opens closed when the window narrows again.
  useEffect(() => {
    const query = window.matchMedia?.(phoneQuery);
    const changed = () => {
      if (!query!.matches) setOpen(false);
    };
    query?.addEventListener("change", changed);
    return () => query?.removeEventListener("change", changed);
  }, []);

  useEffect(() => {
    if (!menuOpen) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") close(true);
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [menuOpen]);

  const choose = (key: string) => {
    onNavigate?.(key);
    if (menuOpen) close(true);
  };

  const link = (item: ShellLink, sub: boolean, marked: boolean) => (
    <a
      href={item.href}
      className={sub ? styles.item : styles.section}
      aria-current={item.key === current ? "page" : marked ? "true" : undefined}
      onClick={() => choose(item.key)}
    >
      <span className={styles.label}>{item.label}</span>
      {/* A screen reader hears "Queues 2 waiting". */}
      {item.badge && " "}
      {item.badge &&
        (sub ? (
          <span className={`${styles.count} ${tones[item.badge.tone]}`}>
            {item.badge.count}
            <span className={a11y.visuallyHidden}> {item.badge.label}</span>
          </span>
        ) : (
          <CountBadge
            tone={item.badge.tone}
            count={item.badge.count}
            label={item.badge.label}
          />
        ))}
    </a>
  );

  return (
    <div className={styles.shell}>
      <header className={styles.side}>
        <div className={styles.top}>
          <button
            ref={menuButton}
            type="button"
            className={styles.menuButton}
            aria-label="Menu"
            aria-expanded={menuOpen}
            aria-controls={panelId}
            onClick={() => (menuOpen ? close(false) : setOpen(true))}
          >
            {menuOpen ? <CrossIcon size={20} /> : <MenuIcon />}
          </button>
          <a href={home} className={styles.logo}>
            <LogoIcon />
            <span>skald</span>
          </a>
        </div>
        <div
          id={panelId}
          className={styles.panel}
          data-open={menuOpen || undefined}
        >
          <Dropdown
            stacked
            label="Space"
            options={spaces}
            value={space}
            onChange={onSpaceChange}
          />
          <nav aria-label="Sections">
            <ul className={styles.sections}>
              {sections.map((section) => (
                <li key={section.key}>
                  {link(
                    section,
                    false,
                    section.items?.some((item) => item.key === current) ??
                      false,
                  )}
                  {section.items && (
                    <ul className={styles.items}>
                      {section.items.map((item) => (
                        <li key={item.key}>{link(item, true, false)}</li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          </nav>
          <AccountFooter account={account} />
        </div>
      </header>
      {menuOpen && (
        <div
          className={styles.backdrop}
          aria-hidden="true"
          onClick={() => close(false)}
        />
      )}
      <main className={styles.page} inert={menuOpen}>
        {children}
      </main>
    </div>
  );
}

/**
 * The foot of the side menu: the initials, the name and the sign-in method, and the menu of account
 * actions. `defaultOpen` opens the menu on the first render, as on the kit page.
 */
export function AccountFooter({
  account,
  defaultOpen,
}: {
  account: ShellAccount;
  defaultOpen?: boolean;
}) {
  return (
    <div className={styles.account}>
      <span className={styles.avatar} aria-hidden="true">
        {account.initials}
      </span>
      <span className={styles.who}>
        <span className={styles.name}>{account.name}</span>
        <span className={styles.signIn}>{account.signIn}</span>
      </span>
      <Menu
        label="Account"
        icon={<MoreIcon />}
        actions={account.actions}
        defaultOpen={defaultOpen}
      />
    </div>
  );
}

/** Whether the window is at phone width; never in a browser without `matchMedia`. */
function usePhoneWidth(): boolean {
  return useSyncExternalStore(
    (changed) => {
      const query = window.matchMedia?.(phoneQuery);
      query?.addEventListener("change", changed);
      return () => query?.removeEventListener("change", changed);
    },
    () => window.matchMedia?.(phoneQuery).matches ?? false,
  );
}
