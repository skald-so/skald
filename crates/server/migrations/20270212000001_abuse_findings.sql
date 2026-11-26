-- Private review findings from existing account-level counter summaries.
CREATE TABLE abuse_findings (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    rule_version text NOT NULL,
    signal text NOT NULL,
    window_seconds integer NOT NULL CHECK (window_seconds > 0),
    minimum_count bigint NOT NULL CHECK (minimum_count > 0),
    evidence jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    retention_seconds integer NOT NULL CHECK (retention_seconds >= window_seconds),
    expires_at timestamp GENERATED ALWAYS AS (
        (created_at AT TIME ZONE 'UTC') + make_interval(secs => retention_seconds)
    ) STORED,
    UNIQUE (account_id, rule_version, signal, window_seconds, minimum_count)
);
CREATE INDEX abuse_findings_expires_at ON abuse_findings (expires_at);
