-- Counting the messages a space stores per calendar month in UTC, for the server's monthly limit
-- (docs/dev/architecture.md, "Cloud limits").

-- The number of messages a space stored in the month that starts on `month`.
CREATE TABLE message_counts (
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    month date NOT NULL,
    messages bigint NOT NULL CHECK (messages >= 1),
    PRIMARY KEY (space_id, month)
);

-- A counted message: its sending device, its msg_id and its space, and when it was counted. Its
-- other envelopes count no more. Kept as long as the idempotency keys of its sends.
CREATE TABLE counted_messages (
    sender_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    msg_id uuid NOT NULL,
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    counted_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (sender_id, msg_id, space_id)
);
CREATE INDEX counted_messages_counted_at ON counted_messages (counted_at);
