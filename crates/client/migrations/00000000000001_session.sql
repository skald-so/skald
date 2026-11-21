-- The machine's session: the server it signed in to and the session's account, once known. At
-- most one row.
CREATE TABLE session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    server TEXT NOT NULL,
    account TEXT,
    github_login TEXT
) STRICT
