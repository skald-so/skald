-- Invites and their acceptances (docs/protocol/membership.md, "Invite").

-- An invite ID the server issued for a space, and the invite stored under it as the server got it.
-- It ends at `expires_at`: 30 days after its issue until an invite is stored, then the invite's
-- `expires`.
CREATE TABLE invites (
    id uuid PRIMARY KEY,
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    invite bytea,
    expires_at timestamptz NOT NULL
);
CREATE INDEX invites_space_id ON invites (space_id);
CREATE INDEX invites_expires_at ON invites (expires_at);

-- The one acceptance an invite admits, as the server got it, and the account it names.
CREATE TABLE acceptances (
    invite_id uuid PRIMARY KEY REFERENCES invites (id) ON DELETE CASCADE,
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    acceptance bytea NOT NULL,
    stored_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX acceptances_account_id ON acceptances (account_id);

-- How many acceptances a space has had: an instance that did not listen then sends the space's
-- `space-notice` to its admins once it listens again.
ALTER TABLE spaces ADD COLUMN acceptances bigint NOT NULL DEFAULT 0;
