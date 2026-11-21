-- Handing messages over to agents (`docs/protocol/delivery.md`, "Device to agent"): when the
-- device wrote each inbox entry, in Unix seconds, none for one an older client wrote; each agent
-- the device has run as in each space, with its cursor then, so each entry after it is for the
-- agent; and each message that passed the checks, by its entry's `seq`, with its space, its line
-- as the CLI shows it, sealed under the state key, until when it waits for an agent, in Unix
-- seconds, and whether the person's run has yet to show it, with each agent it waits for. A
-- message is kept until the person's run showed it and no agent waits for it.
ALTER TABLE inbox ADD COLUMN written INTEGER;
CREATE TABLE agent (
    space TEXT NOT NULL,
    name TEXT NOT NULL,
    since INTEGER NOT NULL CHECK (since >= 0),
    PRIMARY KEY (space, name)
) STRICT;
CREATE TABLE kept (
    seq INTEGER PRIMARY KEY CHECK (seq >= 1),
    space TEXT NOT NULL,
    line BLOB NOT NULL,
    expires INTEGER NOT NULL,
    person INTEGER NOT NULL CHECK (person IN (0, 1))
) STRICT;
CREATE TABLE handover (
    seq INTEGER NOT NULL,
    agent TEXT NOT NULL,
    PRIMARY KEY (seq, agent)
) STRICT;
