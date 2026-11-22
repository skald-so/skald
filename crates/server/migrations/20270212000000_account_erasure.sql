-- Erasing a deleted account's data (docs/dev/architecture.md, "Data model"): whether the server has
-- erased the account, 30 days after its deletion.
ALTER TABLE accounts
    ADD COLUMN erased boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT accounts_erased_once_deleted CHECK (NOT erased OR deleted_at IS NOT NULL);
CREATE INDEX accounts_to_erase ON accounts (deleted_at) WHERE deleted_at IS NOT NULL AND NOT erased;
CREATE INDEX accounts_erased ON accounts (id) WHERE erased;

-- A space and a channel's group keep no creator once the server erases it.
ALTER TABLE spaces ALTER COLUMN creator_id DROP NOT NULL;
ALTER TABLE channel_groups ALTER COLUMN creator_id DROP NOT NULL;
