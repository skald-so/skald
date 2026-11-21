-- The history through the recovery key: the account record the device took of each member
-- account of its spaces ([`crate::records`]), in CBOR; and, on a device linked with the
-- recovery key, each epoch of a channel whose key a wrap to it gave the device.
CREATE TABLE account_record (
    account TEXT PRIMARY KEY,
    record BLOB NOT NULL
) STRICT;
CREATE TABLE recovered_epoch (
    channel TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 1),
    PRIMARY KEY (channel, epoch)
) STRICT;
