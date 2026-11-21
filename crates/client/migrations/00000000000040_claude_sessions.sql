-- Claude Code sessions over their cross-session socket ([`crate::claude_socket`]): each
-- session a `SessionStart` hook registered, by Claude Code's session ID, in the order of
-- registration, with its agent, the agent's space and folder, its socket, its token and its
-- session file; and, of each kept message that waits for an agent, the session whose socket
-- took it, while it waits for the session file to confirm it, whether a receipt of the
-- session's said it did not reach the model, and the ID of the socket line of its signal.
CREATE TABLE claude_session (
    position INTEGER PRIMARY KEY,
    session TEXT NOT NULL UNIQUE,
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    folder TEXT NOT NULL,
    socket TEXT NOT NULL,
    token TEXT,
    file TEXT NOT NULL
) STRICT;
ALTER TABLE handover ADD COLUMN written TEXT;
ALTER TABLE handover ADD COLUMN failed INTEGER NOT NULL DEFAULT 0 CHECK (failed IN (0, 1));
ALTER TABLE handover ADD COLUMN signal TEXT;
