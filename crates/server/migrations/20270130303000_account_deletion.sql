-- Account deletion (docs/dev/architecture.md, "Data model"): when the account deleted itself. A
-- deleted account has no username, so another account may take it.
ALTER TABLE accounts
    ADD COLUMN deleted_at timestamptz,
    ALTER COLUMN username DROP NOT NULL,
    ADD CONSTRAINT accounts_username_until_deleted CHECK ((username IS NULL) = (deleted_at IS NOT NULL));
