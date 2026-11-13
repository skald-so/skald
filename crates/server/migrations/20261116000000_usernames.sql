-- Usernames: each account has its own username, unique on the server, which its sessions change.
-- These functions are the one way a username is made from a text, such as a GitHub login.

-- Whether no account may take `username`.
CREATE FUNCTION username_reserved(username text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT
RETURN username IN ('skald', 'admin', 'support', 'root');

-- The username `source` makes, a `name` of an agent address: lowercased; each run of characters a
-- `name` does not allow becomes `-`; cut to a `name`'s 63 characters; `-` trimmed at both ends;
-- `user` when nothing is left. Only ASCII letters are lowercased: no other character makes one.
CREATE FUNCTION username_from(source text) RETURNS text
LANGUAGE sql IMMUTABLE STRICT
RETURN coalesce(
    nullif(trim(BOTH '-' FROM left(regexp_replace(lower(source COLLATE "C"), '[^a-z0-9]+', '-', 'g'), 63)), ''),
    'user'
);

-- The first username `source` makes that is free: neither reserved nor any account's. When the one
-- `username_from` gives is not, the first free of `-2`, `-3` and on is appended to it, cut so that
-- the whole fits in 63 characters.
CREATE FUNCTION free_username(source text) RETURNS text
LANGUAGE plpgsql STABLE STRICT
AS $$
DECLARE
    base text := username_from(source);
    candidate text := base;
    number integer := 1;
BEGIN
    WHILE username_reserved(candidate) OR EXISTS (SELECT 1 FROM accounts WHERE username = candidate) LOOP
        number := number + 1;
        candidate := left(base, 63 - length('-' || number)) || '-' || number;
    END LOOP;
    RETURN candidate;
END;
$$;

-- A `name` of an agent address, unique, not reserved; set at sign-up.
ALTER TABLE accounts ADD COLUMN username text UNIQUE
    CHECK (username COLLATE "C" ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$' AND NOT username_reserved(username));

-- Each existing account gets the username its GitHub login makes, the earliest account first.
DO $$
DECLARE
    account record;
BEGIN
    FOR account IN
        SELECT a.id, coalesce(m.login, '') AS login FROM accounts a
        LEFT JOIN LATERAL (
            SELECT login FROM sign_in_methods
            WHERE account_id = a.id AND provider = 'github' ORDER BY created_at LIMIT 1
        ) m ON true
        ORDER BY a.created_at, a.id
    LOOP
        UPDATE accounts SET username = free_username(account.login) WHERE id = account.id;
    END LOOP;
END;
$$;

ALTER TABLE accounts ALTER COLUMN username SET NOT NULL;
