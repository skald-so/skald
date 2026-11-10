import { useState } from "react";
import { Button } from "../ui/Button.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { FilterButtons, type FilterOption } from "../ui/FilterButtons.tsx";
import { Search } from "../ui/Search.tsx";
import styles from "./KitPage.module.css";
import {
  accountColumns,
  accounts,
  accountValues,
  type Account,
} from "./accounts.tsx";
import { SortedTable } from "./KitTables.tsx";

// The filters and search of 13-operator-accounts.png and the view switch of 04-messages.png.

type AccountFilter = "all" | "flagged" | "banned" | "enterprise";

const accountFilters: FilterOption<AccountFilter>[] = [
  { value: "all", label: "All" },
  { value: "flagged", label: "Flagged" },
  { value: "banned", label: "Banned" },
  { value: "enterprise", label: "Enterprise" },
];

const inFilter: Record<AccountFilter, (account: Account) => boolean> = {
  all: () => true,
  flagged: (account) => account.status === "Flagged",
  banned: (account) => account.status === "Banned",
  enterprise: (account) => account.plan === "Enterprise",
};

/** Whether the account's name or login, an email for some, contains the query, in any case. */
function found(account: Account, query: string): boolean {
  const words = query.trim().toLowerCase();
  return [account.name, account.handle].some((text) =>
    text.toLowerCase().includes(words),
  );
}

type View = "thread" | "graph";

const views: FilterOption<View>[] = [
  { value: "thread", label: "Thread" },
  { value: "graph", label: "Graph" },
];

export function KitFilters() {
  return (
    <section
      aria-labelledby="kit-filters"
      className={[styles.section, styles.wide].join(" ")}
    >
      <h2 id="kit-filters">Filter buttons and search</h2>
      <FilteredAccounts />
      <ul className={styles.cards}>
        <li className={styles.card}>
          <span className={styles.name}>
            Joined <span className={styles.use}>· as the view of Messages</span>
          </span>
          <MessagesBar />
        </li>
        <li className={styles.card}>
          <span className={styles.name}>
            With a query{" "}
            <span className={styles.use}>· a button clears it</span>
          </span>
          <QuerySample />
        </li>
      </ul>
    </section>
  );
}

function FilteredAccounts() {
  const [filter, setFilter] = useState<AccountFilter>("all");
  const [query, setQuery] = useState("");
  const rows = accounts.filter(
    (account) => inFilter[filter](account) && found(account, query),
  );

  return (
    <div className={styles.tables}>
      <span className={styles.caption}>
        Narrowing a table · as operator accounts
      </span>
      <div className={styles.toolbar}>
        <FilterButtons
          label="Filter accounts"
          options={accountFilters}
          value={filter}
          onChange={setFilter}
        />
        <Search
          label="Search accounts"
          placeholder="Search by name, login or email"
          value={query}
          onChange={setQuery}
        />
      </div>
      <SortedTable
        label="Filtered accounts"
        columns={accountColumns}
        rows={rows}
        rowKey={(account) => account.name}
        values={accountValues}
        initial={{ column: "messages", direction: "descending" }}
        empty={
          <EmptyState
            action={
              <Button
                onClick={() => {
                  setFilter("all");
                  setQuery("");
                }}
              >
                Show all accounts
              </Button>
            }
          >
            No accounts match.
          </EmptyState>
        }
        minWidth={980}
      />
    </div>
  );
}

function MessagesBar() {
  const [view, setView] = useState<View>("thread");
  const [query, setQuery] = useState("");

  return (
    <>
      <div className={styles.toolbar}>
        <FilterButtons
          label="View"
          options={views}
          value={view}
          onChange={setView}
          joined
        />
        <Search
          label="Search messages"
          placeholder="Search in this space"
          value={query}
          onChange={setQuery}
          width={240}
        />
      </div>
      <span className={styles.caption}>
        Showing the {view}
        {query && ` · searching for “${query}”`}
      </span>
    </>
  );
}

function QuerySample() {
  const [query, setQuery] = useState("ana");
  return (
    <>
      <Search
        label="Search members"
        placeholder="Search by name or login"
        value={query}
        onChange={setQuery}
        width={260}
      />
      <span className={styles.caption}>
        {query ? `Query “${query}”` : "No query"}
      </span>
    </>
  );
}
