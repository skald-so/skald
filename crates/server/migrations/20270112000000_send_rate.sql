-- Counting a device's messages within the send rate window (docs/dev/architecture.md, "Cloud
-- limits").
CREATE INDEX counted_messages_sender_counted_at ON counted_messages (sender_id, counted_at);
