-- Sign-in methods: an account is reached through its sign-in methods, not its GitHub ID.

-- A sign-in method: a provider, `github` or an OIDC provider's issuer, and the subject it gives the
-- person, for GitHub the user's ID in decimal. It belongs to one account. The account's key is
-- checked at commit: sign-up adds the method before its account.
CREATE TABLE sign_in_methods (
    provider text NOT NULL CHECK (provider <> ''),
    subject text NOT NULL CHECK (subject <> ''),
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    -- A GitHub method's GitHub login, for display, updated at each sign-in; none for other providers.
    login text CHECK ((provider = 'github') = (login IS NOT NULL)),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX sign_in_methods_account_id ON sign_in_methods (account_id);

-- Each existing account's GitHub ID becomes its GitHub method.
INSERT INTO sign_in_methods (provider, subject, account_id, login, created_at)
SELECT 'github', github_id::text, id, github_login, created_at FROM accounts;

ALTER TABLE accounts DROP COLUMN github_id, DROP COLUMN github_login;
