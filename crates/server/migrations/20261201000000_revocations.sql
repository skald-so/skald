-- Revocations (docs/protocol/membership.md, "Revocation"): each account's counting revocations as
-- the server got them, and which of its devices they revoke.

-- A revocation as the server got it, once per account and bytes: `digest` is the SHA-256 of
-- `revocation`.
CREATE TABLE revocations (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    revocation bytea NOT NULL,
    digest bytea NOT NULL CHECK (length(digest) = 32),
    stored_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (account_id, digest)
);

-- Whether checks 3 and 4 of "Device checks" fail for the device with its account's revocations;
-- set each time the account stores one. A revoked device has no certificate wherever the server
-- checks one.
ALTER TABLE devices ADD COLUMN revoked boolean NOT NULL DEFAULT false;
