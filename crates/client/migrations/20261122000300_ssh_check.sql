-- The checks of certificates' SSH signatures against GitHub ([`crate::ssh_checks`]): by the
-- address of GitHub's API and the SHA-256 of a certificate's body, the login GitHub gave for a
-- signature that passed, or NULL for one that failed.
CREATE TABLE ssh_check (
    api TEXT NOT NULL,
    body BLOB NOT NULL CHECK (length(body) = 32),
    login TEXT,
    PRIMARY KEY (api, body)
) STRICT
