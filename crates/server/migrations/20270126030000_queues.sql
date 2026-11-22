-- Channels' queues and their tasks (docs/protocol/queues.md, "Queues", "Tasks" and "Task states").

-- A queue of a channel with its settings, and the account whose device created it; NULL for
-- `default`, which has no creator.
CREATE TABLE queues (
    id uuid PRIMARY KEY,
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    name text NOT NULL,
    mode text NOT NULL CHECK (mode IN ('at-least-once', 'at-most-once')),
    lease integer NOT NULL CHECK (lease BETWEEN 30 AND 3600),
    max_attempts integer NOT NULL CHECK (max_attempts BETWEEN 1 AND 100),
    creator_id uuid REFERENCES accounts (id) ON DELETE CASCADE,
    UNIQUE (channel_id, name)
);

-- Every channel from before queues gets `default`, with the default settings. Its ID is a UUIDv7:
-- a random UUID with the time in milliseconds in its first 6 bytes and version bits 0111, as
-- PostgreSQL before 18 has no uuidv7().
INSERT INTO queues (id, channel_id, name, mode, lease, max_attempts)
SELECT encode(set_bit(set_bit(overlay(uuid_send(gen_random_uuid())
           PLACING substring(int8send(floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint)
               FROM 3)
           FROM 1 FOR 6), 52, 1), 53, 1), 'hex')::uuid,
       id, 'default', 'at-least-once', 300, 5
FROM channels;

-- A task: the `queued-task` as its device uploaded it, in the order the server stored them, with
-- that device, its `idempotency_key` and its state.
CREATE TABLE tasks (
    seq bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    id uuid PRIMARY KEY,
    queue_id uuid NOT NULL REFERENCES queues (id) ON DELETE CASCADE,
    uploader_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    queued_task bytea NOT NULL,
    idempotency_key bytea,
    state text NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'claimed', 'done', 'dead', 'cancelled')),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0)
);
CREATE UNIQUE INDEX tasks_queue_idempotency_key ON tasks (queue_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX tasks_queue ON tasks (queue_id, seq);

-- The answer to each device's first upload of each task ID that the server stored or refused for a
-- problem of queues.md: a retry gets it again, whatever its bytes. `problem` is NULL for a stored
-- task; `duplicate` names the task of a `duplicate`.
CREATE TABLE task_uploads (
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    task_id uuid NOT NULL,
    problem text,
    duplicate uuid,
    PRIMARY KEY (device_id, task_id)
);
