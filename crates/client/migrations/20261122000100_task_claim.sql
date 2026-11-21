-- The claims the device holds ([`crate::claims`]), by task ID, with their space, channel and
-- queue, the agent that holds each, none for the person, when the device sent the last request
-- the server answered that started its lease, by the device's clock, and the rest of it sealed
-- under the state key: its claim, what its result needs, and its complete or fail, if any.
CREATE TABLE task_claim (
    task TEXT PRIMARY KEY,
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    queue TEXT NOT NULL,
    agent TEXT,
    since INTEGER NOT NULL CHECK (since >= 0),
    sealed BLOB NOT NULL
) STRICT
