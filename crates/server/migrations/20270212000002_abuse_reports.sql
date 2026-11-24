-- Structured reports about a member of a space. A retry is scoped to the reporter and space.
CREATE TABLE abuse_reports (
    receipt text PRIMARY KEY CHECK (length(receipt) = 43),
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    reporter_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    target_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    reason text NOT NULL,
    idempotency_hash bytea NOT NULL,
    submission_hash bytea NOT NULL,
    submitted_at timestamptz NOT NULL DEFAULT statement_timestamp(),
    retention_seconds bigint NOT NULL CHECK (retention_seconds > 0),
    expires_at timestamp GENERATED ALWAYS AS (
        (submitted_at AT TIME ZONE 'UTC') + make_interval(secs => retention_seconds)
    ) STORED,
    CHECK (reporter_id <> target_id),
    UNIQUE (space_id, reporter_id, idempotency_hash)
);
CREATE INDEX abuse_reports_target ON abuse_reports (target_id);
CREATE INDEX abuse_reports_expires_at ON abuse_reports (expires_at);

-- Counts submissions over a day independently of the report's retention.
CREATE TABLE abuse_report_submissions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    reporter_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    submitted_at timestamptz NOT NULL
);
CREATE INDEX abuse_report_submissions_reporter_time
    ON abuse_report_submissions (reporter_id, submitted_at);
CREATE INDEX abuse_report_submissions_time ON abuse_report_submissions (submitted_at);
