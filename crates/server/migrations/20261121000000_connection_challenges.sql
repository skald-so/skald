-- Connection challenges: a device session's challenge, which one WebSocket's first frame signs to
-- prove its device (docs/protocol/delivery.md, "Connecting").

-- A challenge, by its SHA-256, until a connection uses it or it expires. It ends with its session.
CREATE TABLE connection_challenges (
    challenge_hash bytea PRIMARY KEY CHECK (length(challenge_hash) = 32),
    session_hash bytea NOT NULL REFERENCES sessions (token_hash) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE INDEX connection_challenges_session_hash ON connection_challenges (session_hash);
CREATE INDEX connection_challenges_expires_at ON connection_challenges (expires_at);
