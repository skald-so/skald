-- Of each permission prompt (`crate::permission_prompts`): the session's folder, if known, and
-- when it was answered or dropped, in Unix seconds, once it was.
ALTER TABLE permission_prompt ADD COLUMN folder TEXT;
ALTER TABLE permission_prompt ADD COLUMN ended INTEGER CHECK (ended >= 0);
