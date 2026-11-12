-- The device flow (RFC 8628): its codes, the wrong user codes each account typed, and its sessions.

-- A session is a web session, in a browser's cookie, or a device session, a device flow's bearer
-- token; each kind is accepted only where it is sent.
ALTER TABLE sessions ADD COLUMN kind text NOT NULL DEFAULT 'web' CHECK (kind IN ('web', 'device'));
ALTER TABLE sessions ALTER COLUMN kind DROP DEFAULT;

-- A device code and its user code, each by its SHA-256. `pending` until its person approves or
-- denies the user code, `issued` once an approved code gave its session token.
CREATE TABLE device_codes (
    device_code_hash bytea PRIMARY KEY CHECK (length(device_code_hash) = 32),
    user_code_hash bytea NOT NULL UNIQUE CHECK (length(user_code_hash) = 32),
    expires_at timestamptz NOT NULL,
    -- The least number of seconds between polls.
    poll_interval integer NOT NULL CHECK (poll_interval > 0),
    polled_at timestamptz,
    state text NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'approved', 'denied', 'issued')),
    -- The account that approved or denied it.
    account_id uuid REFERENCES accounts (id) ON DELETE CASCADE,
    CHECK ((state = 'pending') = (account_id IS NULL))
);
CREATE INDEX device_codes_expires_at ON device_codes (expires_at);

-- A wrong, used or expired user code an account typed on the verification page.
CREATE TABLE wrong_user_codes (
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    typed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX wrong_user_codes_account_id ON wrong_user_codes (account_id, typed_at);
