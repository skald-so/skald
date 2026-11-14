-- Personal spaces: each account creates and owns at most one, named from its username. A name is
-- numbered one way, for usernames and spaces alike.

-- `base`, a `name`, as the `number`th candidate for a free one: `base` itself for 1, otherwise
-- `base` with `-<number>` appended, cut so that the whole fits in 63 characters.
CREATE FUNCTION numbered_name(base text, number integer) RETURNS text
LANGUAGE sql IMMUTABLE STRICT
RETURN CASE WHEN number = 1 THEN base
    ELSE left(base, 63 - length('-' || number)) || '-' || number END;

CREATE OR REPLACE FUNCTION free_username(source text) RETURNS text
LANGUAGE plpgsql STABLE STRICT
AS $$
DECLARE
    base text := username_from(source);
    number integer := 1;
BEGIN
    WHILE username_reserved(numbered_name(base, number))
        OR EXISTS (SELECT 1 FROM accounts WHERE username = numbered_name(base, number)) LOOP
        number := number + 1;
    END LOOP;
    RETURN numbered_name(base, number);
END;
$$;

-- The first name `username` gives that no space has: `username`, or the first free of `-2`, `-3`
-- and on appended, as `free_username` numbers a taken username.
CREATE FUNCTION free_space_name(username text) RETURNS text
LANGUAGE plpgsql STABLE STRICT
AS $$
DECLARE
    number integer := 1;
BEGIN
    WHILE EXISTS (SELECT 1 FROM spaces WHERE name = numbered_name(username, number)) LOOP
        number := number + 1;
    END LOOP;
    RETURN numbered_name(username, number);
END;
$$;

-- Whether the space is its creator's personal space.
ALTER TABLE spaces ADD COLUMN personal boolean NOT NULL DEFAULT false;
CREATE UNIQUE INDEX spaces_personal ON spaces (creator_id) WHERE personal;
