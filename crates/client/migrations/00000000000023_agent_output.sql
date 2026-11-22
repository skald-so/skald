-- Compact output for agents: the agent each message the device sent is from, none for the
-- person's; the `msg_id` of each kept message, none for one an older client kept; each status
-- of the device's messages, in order: one a run took out of the inbox, by its entry's `seq`,
-- or the server's refusal of a message an agent sent, as JSON (`forms::Status`), with its
-- message's ID and space, if known, whether the person's run has yet to show it and the agent
-- that sent the message until the agent's run took it; and each warning an agent's run gave,
-- as its full line, until the person's run shows it.
ALTER TABLE sent ADD COLUMN agent TEXT;
ALTER TABLE kept ADD COLUMN msg_id TEXT;
CREATE TABLE kept_status (
    position INTEGER PRIMARY KEY,
    seq INTEGER UNIQUE CHECK (seq >= 1),
    msg_id TEXT NOT NULL,
    space TEXT,
    status TEXT NOT NULL,
    person INTEGER NOT NULL CHECK (person IN (0, 1)),
    agent TEXT
) STRICT;
CREATE TABLE kept_warning (
    position INTEGER PRIMARY KEY,
    line TEXT NOT NULL
) STRICT;
