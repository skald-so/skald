-- Presence (`docs/protocol/agents.md`, "Presence"): of each agent of the device in each space,
-- whether it set itself busy and its note, the state the device last gave it and since when,
-- and the runtime of its last MCP session; and of each channel, the last `revision` the device
-- gave a value there, and of the last value the server took, its epoch, `expires` and agents,
-- as JSON.
CREATE TABLE agent_status (
    space TEXT NOT NULL,
    name TEXT NOT NULL,
    busy INTEGER NOT NULL DEFAULT 0 CHECK (busy IN (0, 1)),
    note TEXT,
    state INTEGER CHECK (state BETWEEN 1 AND 4),
    since INTEGER CHECK (since >= 0),
    runtime TEXT,
    PRIMARY KEY (space, name)
) STRICT;
CREATE TABLE presence (
    channel TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    epoch INTEGER CHECK (epoch >= 1),
    expires INTEGER CHECK (expires >= 0),
    agents TEXT
) STRICT;
