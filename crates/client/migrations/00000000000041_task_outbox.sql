-- The tasks the device enqueued that the server has not answered yet, in the order they went
-- in, by task ID, with their space, channel and queue and the `queued-task` every upload of
-- each carries.
CREATE TABLE task_outbox (
    position INTEGER PRIMARY KEY,
    task TEXT NOT NULL UNIQUE,
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    queue TEXT NOT NULL,
    record BLOB NOT NULL
) STRICT
