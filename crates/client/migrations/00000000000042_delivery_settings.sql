-- Delivery settings ([`crate::delivery`]): when the device kept each message, in Unix
-- milliseconds, none for one an older client kept; the mode, `now`, `batch` with its window in
-- seconds, or `pull`, and the text setting, `signal` or `inline`, the person set for each
-- channel and sender trust level; the wakes the device caused in each channel; and, until the
-- session confirms one of them or says none reached the model, the lines of each write to a
-- Claude Code session's socket and the channels whose wake it counted.
ALTER TABLE kept ADD COLUMN came INTEGER CHECK (came >= 0);
CREATE TABLE delivery_setting (
    channel TEXT NOT NULL,
    level TEXT NOT NULL CHECK (level IN ('own', 'peer', 'service')),
    mode TEXT NOT NULL DEFAULT 'now' CHECK (mode IN ('now', 'batch', 'pull')),
    window INTEGER CHECK (window >= 1),
    text TEXT NOT NULL DEFAULT 'signal' CHECK (text IN ('signal', 'inline')),
    PRIMARY KEY (channel, level)
) STRICT;
CREATE TABLE channel_wakes (
    channel TEXT PRIMARY KEY,
    wakes INTEGER NOT NULL CHECK (wakes >= 0)
) STRICT;
CREATE TABLE socket_line (
    line TEXT NOT NULL,
    session TEXT NOT NULL,
    write TEXT NOT NULL,
    PRIMARY KEY (session, line)
) STRICT;
CREATE TABLE socket_wake (
    write TEXT NOT NULL,
    channel TEXT NOT NULL,
    PRIMARY KEY (write, channel)
) STRICT;
