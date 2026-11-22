-- What the device checks the results of each task it enqueued against ([`crate::results`]): its
-- channel and queue, its content's `from` and `to`, if any, and the `msg_id` of the `done` result
-- it took as the task's, once it took one. A task enqueued before has none of them.
ALTER TABLE enqueued_task ADD COLUMN channel TEXT;
ALTER TABLE enqueued_task ADD COLUMN queue TEXT;
ALTER TABLE enqueued_task ADD COLUMN author TEXT;
ALTER TABLE enqueued_task ADD COLUMN addressee TEXT;
ALTER TABLE enqueued_task ADD COLUMN result TEXT;
