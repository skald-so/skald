-- The presence directory ([`crate::presence`]): the highest `revision` the device accepted of
-- each device's presence value in each channel, with the SHA-256 of that value's bytes; and
-- each line an agent's next tool result says, by the agent's space and name, until it says it.
CREATE TABLE presence_revision (
    channel TEXT NOT NULL,
    device TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    hash BLOB NOT NULL CHECK (length(hash) = 32),
    PRIMARY KEY (channel, device)
) STRICT;
CREATE TABLE agent_notice (
    position INTEGER PRIMARY KEY,
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    line TEXT NOT NULL
) STRICT;
