-- The channels whose group the device joins by itself even though the latest GroupInfo lists
-- it: one whose Welcome it refused or whose Commit it could not process, until it is in the
-- group of the channel's current epoch.
CREATE TABLE rejoin (
    channel TEXT PRIMARY KEY
) STRICT
