-- A device code is `refused` once the sign-in on the verification page that would have approved it
-- was refused as a sign-up, while sign-up is closed; no account decided it.
ALTER TABLE device_codes DROP CONSTRAINT device_codes_state_check;
ALTER TABLE device_codes DROP CONSTRAINT device_codes_check;
ALTER TABLE device_codes ADD CONSTRAINT device_codes_state_check
    CHECK (state IN ('pending', 'approved', 'denied', 'refused', 'issued'));
ALTER TABLE device_codes ADD CONSTRAINT device_codes_check
    CHECK ((state IN ('pending', 'refused')) = (account_id IS NULL));
