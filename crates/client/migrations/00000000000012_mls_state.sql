-- The device's MLS state: each item by its name, sealed under the state key in key storage.
CREATE TABLE mls_state (
    name BLOB PRIMARY KEY,
    sealed BLOB NOT NULL
) STRICT
