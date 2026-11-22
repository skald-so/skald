-- Permission prompts Claude Code sessions wait on (`crate::permission_prompts`): each one a
-- hook told, in order, with its agent, the agent's space, the session's ID, the turn it came
-- in, Claude Code's `prompt_id`, when the device recorded it, in Unix seconds, whether no later
-- hook of the session told that it ended, and whether the person's run has shown it.
CREATE TABLE permission_prompt (
    position INTEGER PRIMARY KEY,
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    session TEXT NOT NULL,
    turn TEXT,
    at INTEGER NOT NULL CHECK (at >= 0),
    open INTEGER NOT NULL DEFAULT 1 CHECK (open IN (0, 1)),
    shown INTEGER NOT NULL DEFAULT 0 CHECK (shown IN (0, 1))
) STRICT
