-- Link code runs of a new device without a session, and a device's session by its key
-- (docs/protocol/membership.md, "Link code" and "Device key session").

-- A run without a session names a device that may be registered on no account yet: it is no
-- account's until a session enters its pairing, and its new device reaches it by its token. Such a
-- run is open while it has not expired and its device, if registered, has no certificate and is
-- of the run's account; the sweep deletes the others.
ALTER TABLE link_runs
    DROP CONSTRAINT link_runs_device_id_fkey,
    ALTER COLUMN account_id DROP NOT NULL,
    ADD COLUMN pairing text UNIQUE,
    ADD COLUMN token_hash bytea UNIQUE,
    -- The later messages its token sent.
    ADD COLUMN sent integer NOT NULL DEFAULT 0,
    ADD CHECK ((pairing IS NULL) = (token_hash IS NULL)),
    ADD CHECK (account_id IS NOT NULL OR pairing IS NOT NULL);

-- A pairing an account entered that the server did not hold.
CREATE TABLE wrong_pairings (
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    entered_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX wrong_pairings_account_id ON wrong_pairings (account_id, entered_at);

-- A challenge for a device's session by its key, by its SHA-256, until it expires. `request` is the
-- SHA-256 of the request that used it, which may come again after a lost answer.
CREATE TABLE device_challenges (
    challenge_hash bytea PRIMARY KEY,
    device_id uuid NOT NULL,
    expires_at timestamptz NOT NULL,
    request bytea
);
CREATE INDEX device_challenges_expires_at ON device_challenges (expires_at);

-- The device whose key gave a device session; a device has one such session at a time.
ALTER TABLE sessions ADD COLUMN key_device uuid UNIQUE REFERENCES devices (id) ON DELETE CASCADE;
