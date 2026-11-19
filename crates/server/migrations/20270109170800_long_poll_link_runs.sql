-- The latest link code run of its account that a device's long-poll answers have covered: a
-- long-poll answers a `link-notice` once a later run is open (docs/protocol/delivery.md, "Mailbox").
-- No foreign key to the run, which ends and is deleted while the device keeps its row.
CREATE TABLE long_poll_link_runs (
    device_id uuid PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    run_id bigint NOT NULL
);
