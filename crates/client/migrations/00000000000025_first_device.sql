-- The account's first device: the account record and the master backup it sends with the
-- master key while its link is `sent`, in CBOR; and, while a recovery key waits in the
-- secrets to be shown to the person, the server it was made on, which sign-out keeps.
ALTER TABLE device ADD COLUMN record BLOB;
ALTER TABLE device ADD COLUMN backup BLOB;
ALTER TABLE device ADD COLUMN recovery TEXT;
