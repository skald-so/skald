-- Key changes the server makes ([`crate::kept_certs`], [`crate::follow::master_warnings`]): the
-- certificate of each device the device trusted, in CBOR, the first it trusted first, then each
-- of another body that passes the proof check; of each such device given with another
-- certificate, the one it last warned of; and each set of master keys of one account it warned
-- of, as text.
CREATE TABLE kept_cert (
    device TEXT NOT NULL,
    cert BLOB NOT NULL,
    PRIMARY KEY (device, cert)
) STRICT;
CREATE TABLE swapped_cert (
    device TEXT PRIMARY KEY,
    cert BLOB NOT NULL
) STRICT;
CREATE TABLE master_warning (
    masters TEXT PRIMARY KEY
) STRICT;
