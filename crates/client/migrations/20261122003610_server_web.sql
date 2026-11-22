-- The web address of each server, an origin, as its device flow gave it: invite links start with
-- it ([`crate::acceptances`]).
CREATE TABLE server_web (
    server TEXT PRIMARY KEY,
    web TEXT NOT NULL
) STRICT;
