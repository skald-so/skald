-- Claims, leases and the DLQ (docs/protocol/queues.md, "Task states", "Claims and leases" and
-- "Modes and retries").

-- A task's scheduling fields the server matches claims by, its last claim and what the state
-- carries: the progress of the holder's last extend, the reason of a `dead` task, releases since
-- enqueue or the last restart, when its lease and its backoff end, and when it got its state.
-- Tasks stored before have no scheduling fields here: no client enqueued any.
ALTER TABLE tasks
    ADD COLUMN template text,
    ADD COLUMN addressee uuid,
    ADD COLUMN deadline timestamptz,
    ADD COLUMN generation bigint NOT NULL DEFAULT 0 CHECK (generation >= 0),
    ADD COLUMN releases integer NOT NULL DEFAULT 0 CHECK (releases >= 0),
    ADD COLUMN claim_id uuid,
    ADD COLUMN claim_device uuid,
    ADD COLUMN lease_until timestamptz,
    ADD COLUMN not_before timestamptz,
    ADD COLUMN progress text CHECK (progress IN ('started', 'awaiting-approval')),
    ADD COLUMN reason text
        CHECK (reason IN ('lease-expired', 'failed', 'released', 'deadline-passed')),
    ADD COLUMN since timestamptz NOT NULL DEFAULT now();
CREATE INDEX tasks_queued ON tasks (queue_id, seq) WHERE state = 'queued';
CREATE INDEX tasks_leases ON tasks (lease_until) WHERE state = 'claimed';
CREATE INDEX tasks_deadlines ON tasks (deadline) WHERE state = 'queued' AND deadline IS NOT NULL;
CREATE INDEX tasks_finished ON tasks (since) WHERE state IN ('done', 'dead', 'cancelled');

-- Each claim of a task: its generation, the device holding it, its lease in seconds, and the act
-- the server took that ended it, which a repeat of that act gets again.
CREATE TABLE claims (
    id uuid PRIMARY KEY,
    task_id uuid NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    generation bigint NOT NULL CHECK (generation >= 1),
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    lease integer NOT NULL,
    ended text CHECK (ended IN ('complete', 'fail', 'release'))
);
CREATE INDEX claims_task ON claims (task_id);

-- Until when a task is not handed to a device that released it.
CREATE TABLE task_waits (
    task_id uuid NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    until timestamptz NOT NULL,
    PRIMARY KEY (task_id, device_id)
);
