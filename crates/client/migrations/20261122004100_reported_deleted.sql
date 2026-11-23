-- The member accounts of each space the server reports deleted that the device told its user of
-- ([`crate::follow`]); an account the server no longer reports goes, so a later report is told
-- again.
CREATE TABLE reported_deleted (
    space TEXT NOT NULL,
    account TEXT NOT NULL,
    PRIMARY KEY (space, account)
) STRICT;
