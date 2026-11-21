-- Account deletion (docs/dev/architecture.md, "Data model"): when the account deleted itself. A
-- deleted account has no username, so another account may take it.
ALTER TABLE accounts
    ADD COLUMN deleted_at timestamptz,
    ALTER COLUMN username DROP NOT NULL,
    ADD CONSTRAINT accounts_username_until_deleted CHECK ((username IS NULL) = (deleted_at IS NOT NULL));

-- How many times an account of a space's latest revision was deleted: an instance that did not
-- listen then sends the space's `space-notice` once it listens again.
ALTER TABLE spaces ADD COLUMN member_deletions bigint NOT NULL DEFAULT 0;
