-- The rules that bind a folder and everything below it to a space: the folder's canonical path
-- and the space's name, which may be of a space the device is not in.
CREATE TABLE folder_rule (
    folder TEXT PRIMARY KEY,
    space TEXT NOT NULL
) STRICT
