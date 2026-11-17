import { useContext, useEffect, useState } from "react";
import { listSpaceMembers } from "../api/skald.ts";
import type { ListedMember, ListedMemberRole } from "../api/skald.schemas.ts";
import { Banner } from "../ui/Banner.tsx";
import { Button } from "../ui/Button.tsx";
import { EmptyState } from "../ui/EmptyState.tsx";
import { FilterButtons, type FilterOption } from "../ui/FilterButtons.tsx";
import { PageHeader } from "../ui/PageHeader.tsx";
import { Search } from "../ui/Search.tsx";
import { sortRows, type Sort } from "../ui/sort.ts";
import { Table, TwoLine, type Column } from "../ui/Table.tsx";
import { versioned } from "./api.ts";
import { ChosenSpace } from "./space.ts";
import styles from "./Members.module.css";
import { date } from "./time.ts";

/** Each role for people and its place when sorted. */
const roles: Record<ListedMemberRole, { label: string; order: number }> = {
  admin: { label: "Admin", order: 0 },
  member: { label: "Member", order: 1 },
};

type Filter = "all" | ListedMemberRole;

/** The value of `member` in the sorted column `column`. */
function value(member: ListedMember, column: string): string | number {
  switch (column) {
    case "role":
      return roles[member.role].order;
    case "devices":
      return member.linked_devices;
    case "joined":
      return Date.parse(member.joined_at);
    default:
      return member.handle;
  }
}

/** Whether the handle or the GitHub login of `member` holds `query`, in any case. */
function found(member: ListedMember, query: string): boolean {
  const words = query.trim().toLowerCase();
  return [member.handle, member.github_login ?? ""].some((text) =>
    text.toLowerCase().includes(words),
  );
}

const columns = (now: number): Column<ListedMember>[] => [
  {
    key: "person",
    label: "Person",
    sortable: true,
    width: "minmax(0, 2fr)",
    cell: (member) => (
      <TwoLine
        detail={
          member.github_login !== undefined && `GitHub · ${member.github_login}`
        }
      >
        {member.handle}
      </TwoLine>
    ),
  },
  {
    key: "role",
    label: "Role",
    sortable: true,
    cell: (member) => roles[member.role].label,
  },
  {
    key: "devices",
    label: "Devices",
    sortable: true,
    cell: (member) => member.linked_devices,
  },
  {
    key: "joined",
    label: "Joined",
    sortable: true,
    muted: true,
    cell: (member) => date(member.joined_at, now),
  },
];

export interface MembersScreenProps {
  /** The name of the space the switcher chose; none while the account has no space. */
  space?: string;
  /** The space's members; none while they load. */
  members?: readonly ListedMember[];
  /** The time the dates count from, in milliseconds since the epoch. */
  now: number;
  /** What failed, shown above the table. */
  failed?: string;
}

/**
 * The Members section of 09-members.png: a row per member of the chosen space with its handle and
 * GitHub login, role, linked devices and when it joined, in the server's order until a click on a
 * column sorts them; filter buttons by role and a search by handle or login, in the browser. It
 * only shows them: an admin changes them with `skald`.
 */
export function MembersScreen({
  space,
  members,
  now,
  failed,
}: MembersScreenProps) {
  const [sort, setSort] = useState<Sort>();
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");

  if (space === undefined)
    return (
      <>
        <PageHeader title="Members" />
        <EmptyState>
          No space yet. Sign in a machine with <code>skald</code> to make your
          personal space.
        </EmptyState>
      </>
    );

  const all = members ?? [];
  const count = (role: ListedMemberRole) =>
    all.filter((member) => member.role === role).length;
  const filters: FilterOption<Filter>[] = [
    { value: "all", label: "All", count: all.length },
    { value: "admin", label: "Admins", count: count("admin") },
    { value: "member", label: "Members", count: count("member") },
  ];
  const shown = all.filter(
    (member) =>
      (filter === "all" || member.role === filter) && found(member, query),
  );

  return (
    <>
      <PageHeader
        title="Members"
        subtitle={
          <>
            People in {space} and their roles, as the server lists them;{" "}
            <code>skald</code> checks them itself. An admin changes them with{" "}
            <code>skald</code>.
          </>
        }
      />
      {failed && <Banner>{failed}</Banner>}
      <div className={styles.toolbar}>
        <FilterButtons
          label="Filter members"
          options={filters}
          value={filter}
          onChange={setFilter}
        />
        <Search
          label="Search members"
          placeholder="Search by handle or login"
          value={query}
          onChange={setQuery}
          width={260}
        />
      </div>
      <Table
        label="Members"
        columns={columns(now)}
        rows={sort ? sortRows(shown, sort, value) : shown}
        rowKey={(member) => member.handle}
        sort={sort}
        onSort={setSort}
        loading={!members}
        loadingLabel="Loading members"
        empty={
          all.length === 0 ? (
            <EmptyState>No members yet.</EmptyState>
          ) : (
            <EmptyState
              action={
                <Button
                  onClick={() => {
                    setFilter("all");
                    setQuery("");
                  }}
                >
                  Show all members
                </Button>
              }
            >
              No members match.
            </EmptyState>
          )
        }
        minWidth={560}
      />
    </>
  );
}

/** The members of `space`, or `null` when the API refuses the session; throws when it gives neither. */
async function fetchMembers(space: string): Promise<ListedMember[] | null> {
  const listed = await listSpaceMembers(space, { headers: versioned });
  if (listed.status === 401) return null;
  if (listed.status !== 200)
    throw new Error(`GET /spaces/${space}/members ${listed.status}`);
  return listed.data.members;
}

/**
 * The Members section of the space the switcher chose, with its members from the API. When the API
 * refuses the session, `onSignedOut` shows the sign-in page.
 */
export function Members({ onSignedOut }: { onSignedOut: () => void }) {
  const space = useContext(ChosenSpace);
  const [listed, setListed] = useState<{
    space: string;
    members: ListedMember[];
    at: number;
  }>();
  const [failed, setFailed] = useState<{ space: string }>();

  useEffect(() => {
    if (space === undefined) return;
    // An answer for a space chosen before is dropped.
    let shown = true;
    fetchMembers(space.space).then(
      (members) => {
        if (!shown) return;
        if (!members) return onSignedOut();
        setListed({ space: space.space, members, at: Date.now() });
        setFailed(undefined);
      },
      () => {
        if (shown) setFailed({ space: space.space });
      },
    );
    return () => {
      shown = false;
    };
  }, [space, onSignedOut]);

  const current = listed?.space === space?.space ? listed : undefined;
  return (
    <MembersScreen
      key={space?.space}
      space={space?.name}
      members={current?.members}
      now={current?.at ?? 0}
      failed={
        failed?.space === space?.space && space !== undefined
          ? "The members did not load."
          : undefined
      }
    />
  );
}
