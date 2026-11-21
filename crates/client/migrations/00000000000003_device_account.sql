-- The account the device belongs to: the account of the session it was set up with. A device
-- of a state older than this column takes its session's account, if the session has one.
ALTER TABLE device ADD COLUMN account TEXT;
UPDATE device SET account = (SELECT account FROM session);
