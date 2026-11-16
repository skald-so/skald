-- KeyPackages: each `client` device's KeyPackages for others to add it, and its last-resort one
-- (docs/protocol/membership.md, "KeyPackages").

-- A KeyPackage as its device's account uploaded it, in an `MLSMessage`, and the end of its
-- lifetime. Another one is deleted once handed out; the last-resort one, at most one per device, is
-- marked handed out and kept until a new one replaces it.
CREATE TABLE key_packages (
    id bigserial PRIMARY KEY,
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    last_resort boolean NOT NULL,
    key_package bytea NOT NULL,
    expires_at timestamptz NOT NULL,
    handed_out boolean NOT NULL DEFAULT false CHECK (last_resort OR NOT handed_out)
);
CREATE INDEX key_packages_device_id ON key_packages (device_id, expires_at);
CREATE UNIQUE INDEX key_packages_last_resort ON key_packages (device_id) WHERE last_resort;
-- The same KeyPackage uploaded again is one KeyPackage, handed out once.
CREATE UNIQUE INDEX key_packages_once ON key_packages (device_id, sha256(key_package))
    WHERE NOT last_resort;
