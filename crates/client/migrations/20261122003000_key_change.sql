-- The key changes the device warned of ([`crate::key_changes`]), in order: what tells each apart,
-- its warning, what found it, `github <API's URL>` or `dns <resolver's URL>`, or '' for the rest,
-- and whether its person saw it. Seen ones stay, so a change kept once is not kept again.
CREATE TABLE key_change (
    position INTEGER PRIMARY KEY,
    key TEXT NOT NULL,
    line TEXT NOT NULL,
    source TEXT NOT NULL,
    seen INTEGER NOT NULL DEFAULT 0 CHECK (seen IN (0, 1))
) STRICT
