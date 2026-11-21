-- Replies: each kept message's channel and what a reply to it needs, sealed under the state
-- key; and each message the person's run showed or the device handed to an agent, by its
-- entry's `seq`, with its `msg_id`, space and channel, what a reply needs, sealed, until when
-- it can be replied to, in Unix seconds, whether the person's run showed it, and each agent it
-- was handed to.
ALTER TABLE kept ADD COLUMN channel TEXT;
ALTER TABLE kept ADD COLUMN target BLOB;
CREATE TABLE reply_target (
    seq INTEGER PRIMARY KEY CHECK (seq >= 1),
    msg_id TEXT NOT NULL,
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    target BLOB NOT NULL,
    expires INTEGER NOT NULL,
    person INTEGER NOT NULL CHECK (person IN (0, 1))
) STRICT;
CREATE TABLE reply_agent (
    seq INTEGER NOT NULL,
    agent TEXT NOT NULL,
    PRIMARY KEY (seq, agent)
) STRICT;
