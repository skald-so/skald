-- History retention (docs/protocol/membership.md, "Uploads"): when the server stored each entry,
-- and per channel the earliest epoch whose links and wraps it keeps, 0 until retention deletes an
-- entry. Entries stored before this migration count from it.
ALTER TABLE history_entries ADD COLUMN stored_at timestamptz NOT NULL DEFAULT now();

CREATE INDEX history_entries_stored ON history_entries (stored_at);

ALTER TABLE channel_groups ADD COLUMN history_cutoff bigint NOT NULL DEFAULT 0;
