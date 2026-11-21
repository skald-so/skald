-- Each task the device enqueued, by task ID, with its space and the agent it was enqueued as,
-- none for the person: an agent lists only its own.
CREATE TABLE enqueued_task (
    task TEXT PRIMARY KEY,
    space TEXT NOT NULL,
    agent TEXT
) STRICT
