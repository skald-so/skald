-- Anchors objects (docs/protocol/membership.md, "Anchors"): the last one the server took of each
-- device, as it got it; kept once the device is revoked.
ALTER TABLE devices ADD COLUMN anchors bytea;
