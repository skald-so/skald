-- The latest presence value of each device in each channel, as the device uploaded it
-- (docs/protocol/agents.md, "Presence").
CREATE TABLE presence_values (
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    value bytea NOT NULL,
    PRIMARY KEY (channel_id, device_id)
);

CREATE INDEX presence_values_device ON presence_values (device_id);
