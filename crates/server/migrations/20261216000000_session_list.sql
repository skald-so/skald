-- The account's sessions as its web sessions list them: an ID that reveals nothing of the token,
-- the last use, a web session's browser and OS, and a device session's device.

ALTER TABLE sessions
    ADD COLUMN id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    -- To the minute: a request writes it only when it is a minute old or more.
    ADD COLUMN last_used_at timestamptz,
    -- Read from the `User-Agent` of a web session's sign-in; no IP address or location.
    ADD COLUMN browser text,
    ADD COLUMN os text,
    -- The device a device session registered or connected as, the latest one.
    ADD COLUMN device_id uuid REFERENCES devices (id) ON DELETE SET NULL;
UPDATE sessions SET last_used_at = created_at;
ALTER TABLE sessions
    ALTER COLUMN last_used_at SET NOT NULL,
    ALTER COLUMN last_used_at SET DEFAULT now();
