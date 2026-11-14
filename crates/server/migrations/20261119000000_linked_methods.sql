-- Linked sign-in methods: a sign-in in progress may be a link, and every one has a PKCE verifier.

-- The sign-ins in progress from before have no verifier; each would expire within minutes.
DELETE FROM sign_in_states;
ALTER TABLE sign_in_states
    DROP CONSTRAINT sign_in_states_check,
    DROP CONSTRAINT sign_in_states_check1,
    ALTER COLUMN code_verifier SET NOT NULL,
    ADD CHECK ((provider = 'github') = (nonce IS NULL)),
    -- A link: the web session that started it, by its token's SHA-256. It ends with the session.
    ADD COLUMN link_session bytea REFERENCES sessions (token_hash) ON DELETE CASCADE,
    -- Whether a link's callback has opened itself again to carry the session's cookie.
    ADD COLUMN reopened boolean NOT NULL DEFAULT false;
CREATE INDEX sign_in_states_link_session ON sign_in_states (link_session);
