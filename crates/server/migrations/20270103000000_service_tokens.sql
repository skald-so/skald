-- Tokens of `service` devices for a space: the SHA-256 of the value, never the value, the
-- permissions, and the channels and expiry of the grant, kept as it was sent. A revocation deletes
-- the token, as does a revision that lists its device in no service agent entry; one past its
-- expiry or of a revoked device is refused.

CREATE TABLE service_tokens (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash bytea NOT NULL UNIQUE,
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    permissions text[] NOT NULL CHECK (
        cardinality(permissions) > 0 AND permissions <@ ARRAY['send', 'receive', 'queues']
    ),
    channels uuid[] NOT NULL CHECK (cardinality(channels) > 0),
    grant_cbor bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    -- To the minute: a request writes it only when it is a minute old or more.
    last_used_at timestamptz,
    expires_at timestamptz NOT NULL
);
CREATE INDEX service_tokens_space_id ON service_tokens (space_id, created_at);
CREATE INDEX service_tokens_device_id ON service_tokens (device_id);
