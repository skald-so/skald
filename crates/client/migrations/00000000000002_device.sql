-- The machine's own device: its ID and `name`, set at its first sign-in, whether the server
-- took its registration, and how it is linked: `new` until it knows, `sent` with the master
-- public key it pinned and the certificate it issued with that key until the server takes
-- them, then `certified`; `waiting` for linking by another device. At most one row.
CREATE TABLE device (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    device TEXT NOT NULL,
    name TEXT NOT NULL,
    registered INTEGER NOT NULL DEFAULT 0 CHECK (registered IN (0, 1)),
    link TEXT NOT NULL DEFAULT 'new'
        CHECK (link IN ('new', 'sent', 'certified', 'waiting')),
    master BLOB CHECK (length(master) = 32),
    cert BLOB,
    CHECK ((link IN ('sent', 'certified')) = (master IS NOT NULL AND cert IS NOT NULL)),
    CHECK (link IN ('sent', 'certified') OR (master IS NULL AND cert IS NULL))
) STRICT
