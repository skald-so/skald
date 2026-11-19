-- A challenge is of a device session or of a service token (docs/protocol/delivery.md,
-- "Connecting"), and ends with it: a token's revocation, or a revision that drops its device's
-- service agent entry, deletes the token and so its challenges.

ALTER TABLE connection_challenges
    ALTER COLUMN session_hash DROP NOT NULL,
    ADD COLUMN service_token_hash bytea
        REFERENCES service_tokens (token_hash) ON DELETE CASCADE,
    ADD CONSTRAINT connection_challenges_one_bearer
        CHECK (num_nonnulls(session_hash, service_token_hash) = 1);
CREATE INDEX connection_challenges_service_token_hash
    ON connection_challenges (service_token_hash);
