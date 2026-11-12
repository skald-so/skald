-- Devices: each account's devices by their public keys, and the account's master public key. The
-- server holds no private key.

-- The master public key, which the account's first device gives once.
ALTER TABLE accounts ADD COLUMN master bytea CHECK (length(master) = 32);

-- A device as it registered, by the ID derived from its signing key, and its certificate as it sent
-- it. `name` is its UTF-8 bytes: a `text` cannot hold U+0000.
CREATE TABLE devices (
    id uuid PRIMARY KEY,
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('client', 'browser', 'service')),
    signing bytea NOT NULL CHECK (length(signing) = 32),
    hpke bytea NOT NULL CHECK (length(hpke) = 32),
    name bytea NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    cert bytea,
    registered_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX devices_account_id ON devices (account_id, registered_at);
