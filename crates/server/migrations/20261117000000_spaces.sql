-- Spaces, their roster revisions and the channel IDs the server issued for them.

-- A space by the ID the server issued, and its name, unique on the server. Its creator stores its
-- revision 1.
CREATE TABLE spaces (
    id uuid PRIMARY KEY,
    name text NOT NULL UNIQUE,
    creator_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX spaces_creator_id ON spaces (creator_id);

-- A roster revision as the server got it. Revision n+1 is stored only after revision n, so of two
-- with one number the first is kept.
CREATE TABLE roster_revisions (
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    revision bigint NOT NULL CHECK (revision >= 1),
    roster bytea NOT NULL,
    stored_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (space_id, revision)
);

-- The accounts a revision lists as members, and which of them are admins: who gets the space and
-- stores its next revision. An account may be one this server does not hold.
CREATE TABLE roster_members (
    space_id uuid NOT NULL,
    revision bigint NOT NULL,
    account_id uuid NOT NULL,
    admin boolean NOT NULL,
    PRIMARY KEY (space_id, revision, account_id),
    FOREIGN KEY (space_id, revision) REFERENCES roster_revisions (space_id, revision) ON DELETE CASCADE
);
CREATE INDEX roster_members_account_id ON roster_members (account_id);

-- A channel ID the server issued for a space.
CREATE TABLE channels (
    id uuid PRIMARY KEY,
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    issued_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX channels_space_id ON channels (space_id);

-- The accounts that get each space: the members in its latest revision, and its creator before
-- revision 1; `admin` if the account is an admin in that revision.
CREATE VIEW space_access AS
SELECT s.id AS space_id, s.creator_id AS account_id, false AS admin FROM spaces s
WHERE NOT EXISTS (SELECT 1 FROM roster_revisions r WHERE r.space_id = s.id)
UNION ALL
SELECT m.space_id, m.account_id, m.admin FROM roster_members m
WHERE m.revision = (SELECT max(r.revision) FROM roster_revisions r WHERE r.space_id = m.space_id);
