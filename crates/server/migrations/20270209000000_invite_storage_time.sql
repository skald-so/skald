-- The web app shows when each invite was stored. For earlier invites only the time in the
-- issued ID is known.
ALTER TABLE invites ADD COLUMN created_at timestamptz;
UPDATE invites
SET created_at = to_timestamp(('x' || left(replace(id::text, '-', ''), 12))::bit(48)::bigint / 1000.0)
WHERE invite IS NOT NULL;
ALTER TABLE invites ADD CHECK ((invite IS NULL) = (created_at IS NULL));
