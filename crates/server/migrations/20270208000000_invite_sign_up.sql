-- A verified sign-up proof attaches the invite ID and the key it verified against to a grant.
-- There is no reference to the invite, so deleting it does not remove the grant.
ALTER TABLE device_codes
    ADD COLUMN sign_up_invite uuid,
    ADD COLUMN sign_up_key bytea CHECK (length(sign_up_key) = 32),
    ADD COLUMN sign_up_account_id uuid REFERENCES accounts (id) DEFERRABLE INITIALLY DEFERRED,
    ADD CHECK ((sign_up_invite IS NULL) = (sign_up_key IS NULL));

-- Sign-up gives the invite to one account before it has an acceptance. The account is created in
-- the same transaction.
ALTER TABLE invites ADD COLUMN sign_up_account_id uuid
    REFERENCES accounts (id) DEFERRABLE INITIALLY DEFERRED;
