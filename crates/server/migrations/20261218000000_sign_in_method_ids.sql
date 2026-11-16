-- Each sign-in method has an ID that names it in the request that removes it and reveals nothing of
-- its subject.
ALTER TABLE sign_in_methods ADD COLUMN id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid();
