-- The account record and the master backup (docs/protocol/membership.md, "Account"), each as the
-- first device sent it, with the master key. An account whose master key came before them has
-- neither.
ALTER TABLE accounts
    ADD COLUMN record bytea,
    ADD COLUMN backup bytea,
    ADD CONSTRAINT accounts_backup CHECK (
        (record IS NULL) = (backup IS NULL) AND (record IS NULL OR master IS NOT NULL)
    );
