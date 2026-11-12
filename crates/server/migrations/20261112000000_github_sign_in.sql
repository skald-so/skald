-- GitHub sign-in: accounts, web sessions and sign-ins in progress.

-- An account, created at its GitHub user's first sign-in and bound to the user's GitHub ID.
CREATE TABLE accounts (
    id uuid PRIMARY KEY,
    github_id bigint NOT NULL UNIQUE,
    -- For display, updated at each sign-in.
    github_login text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- A web session, by the SHA-256 of its token: the database never holds the token.
CREATE TABLE sessions (
    token_hash bytea PRIMARY KEY CHECK (length(token_hash) = 32),
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sessions_account_id ON sessions (account_id);

-- A sign-in in progress, by the SHA-256 of its `state`, which works once until it expires.
CREATE TABLE sign_in_states (
    state_hash bytea PRIMARY KEY CHECK (length(state_hash) = 32),
    return_to text NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE INDEX sign_in_states_expires_at ON sign_in_states (expires_at);
