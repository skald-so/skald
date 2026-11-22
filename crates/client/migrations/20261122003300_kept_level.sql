-- Delivery settings on Codex sessions ([`crate::codex`]): each kept message's sender trust level,
-- which Codex's hooks read without the state key; none for one an older client kept from another
-- device, `own` for the device's own, whose `seq` starts at `OWN_FIRST`, 2^62.
ALTER TABLE kept ADD COLUMN level TEXT CHECK (level IN ('own', 'peer', 'service'));
UPDATE kept SET level = 'own' WHERE seq >= 4611686018427387904;
