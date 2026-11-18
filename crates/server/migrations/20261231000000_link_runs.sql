-- Link code runs (docs/protocol/membership.md, "Link code"): the server relays each run's messages
-- as the bytes it got and never sees the code.

-- A device's open run: its hello and its `session`, until it ends. A run is open while it has not
-- expired and its device has no certificate; a device's new hello replaces its run.
CREATE TABLE link_runs (
    id bigserial PRIMARY KEY,
    device_id uuid NOT NULL UNIQUE REFERENCES devices (id) ON DELETE CASCADE,
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    session bytea NOT NULL CHECK (length(session) = 16),
    hello bytea NOT NULL,
    expires_at timestamptz NOT NULL,
    UNIQUE (account_id, session)
);
CREATE INDEX link_runs_expires_at ON link_runs (expires_at);

-- A run's later messages, in the order the server took them.
CREATE TABLE link_messages (
    id bigserial PRIMARY KEY,
    run_id bigint NOT NULL REFERENCES link_runs (id) ON DELETE CASCADE,
    message bytea NOT NULL
);
CREATE INDEX link_messages_run_id ON link_messages (run_id, id);
