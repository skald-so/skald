-- The `.skald.toml` files the device's user approved: the canonical path of each one's folder
-- and the SHA-256 of its bytes as approved.
CREATE TABLE approved_config (
    folder TEXT PRIMARY KEY,
    hash BLOB NOT NULL CHECK (length(hash) = 32)
) STRICT
