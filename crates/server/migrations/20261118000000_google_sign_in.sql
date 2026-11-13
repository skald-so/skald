-- Google sign-in: a sign-in in progress belongs to one provider, and an OIDC provider's keeps its
-- `nonce` and PKCE verifier until its callback.
ALTER TABLE sign_in_states
    ADD COLUMN provider text NOT NULL DEFAULT 'github' CHECK (provider <> ''),
    ADD COLUMN nonce text,
    ADD COLUMN code_verifier text,
    ADD CHECK ((provider = 'github') = (nonce IS NULL AND code_verifier IS NULL)),
    ADD CHECK ((nonce IS NULL) = (code_verifier IS NULL));
ALTER TABLE sign_in_states ALTER COLUMN provider DROP DEFAULT;
