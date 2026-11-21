-- The device's own channel messages, which it keeps for its person and agents once the server
-- stored them (`docs/protocol/delivery.md`, "Device to agent"): the last `seq` it gave one, past
-- [`OWN_FIRST`]; each one whose envelope waits in the outbox, by `msg_id`, with its `seq`, space
-- and channel, its line and what a reply needs, sealed, how long it waits for an agent, whether
-- the person's run shows it, the agent that sent it, whether it has a `to` and the agent of
-- this device that `to` names, and the `replied` it gives, if any; and each kept one whose
-- `read` the device sets once it hands the message to the agent, with that status as JSON.
ALTER TABLE device ADD COLUMN own_seq INTEGER NOT NULL DEFAULT 0 CHECK (own_seq >= 0);
CREATE TABLE own_message (
    msg_id TEXT PRIMARY KEY,
    seq INTEGER NOT NULL UNIQUE CHECK (seq >= 1),
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    line BLOB NOT NULL,
    target BLOB NOT NULL,
    wait INTEGER NOT NULL CHECK (wait >= 0),
    person INTEGER NOT NULL CHECK (person IN (0, 1)),
    sender TEXT,
    addressed INTEGER NOT NULL CHECK (addressed IN (0, 1)),
    addressee TEXT,
    read TEXT,
    replied TEXT,
    replied_status TEXT,
    replied_agent TEXT
) STRICT;
CREATE TABLE own_read (
    seq INTEGER PRIMARY KEY CHECK (seq >= 1),
    msg_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    status TEXT NOT NULL
) STRICT;
