-- A run without a session opens before its hello: the server picks its pairing first, and the
-- hello comes as the run's first message (docs/protocol/membership.md, "Link code"). Until then
-- the run has no `session` and no hello, and no account pairs it.
ALTER TABLE link_runs
    ALTER COLUMN session DROP NOT NULL,
    ALTER COLUMN hello DROP NOT NULL,
    ADD CHECK ((session IS NULL) = (hello IS NULL)),
    ADD CHECK (session IS NOT NULL OR pairing IS NOT NULL);
