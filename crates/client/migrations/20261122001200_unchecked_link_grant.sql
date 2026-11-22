-- Whether a device checked the issuer and the link object of its grant: a device without a session
-- checks them only once its device key session lists the account's certificates, and until then
-- neither stores nor pins the grant (docs/protocol/membership.md, "Link code").
ALTER TABLE link_grant ADD COLUMN checked INTEGER NOT NULL DEFAULT 1 CHECK (checked IN (0, 1));
