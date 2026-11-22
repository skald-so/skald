-- Codex sessions' wakes ([`crate::codex`]): the `codex` of each session's Codex process, if
-- known, and whether a signal the helper queued for it waits for the turn it starts.
ALTER TABLE codex_session ADD COLUMN codex TEXT;
ALTER TABLE codex_session ADD COLUMN queued INTEGER NOT NULL DEFAULT 0 CHECK (queued IN (0, 1));
