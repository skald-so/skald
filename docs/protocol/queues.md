# Skald protocol: queues and tasks

Part of the [Skald protocol](README.md), with its conventions and status. **No external audit.**

CDDL rules not defined here, such as `id`, `content`, `signed` and `timestamp`, are in [README.md](README.md), [delivery.md](delivery.md) and [membership.md](membership.md).

## Contents
- [Queues](#queues)
- [Tasks](#tasks)
- [Queue key](#queue-key)
- [Task states](#task-states)
- [Claims and leases](#claims-and-leases)
- [Modes and retries](#modes-and-retries)
- [Claim checks](#claim-checks)
- [Running a task](#running-a-task)
- [Results](#results)
- [Templates](#templates)
- [Problems](#problems)
- [Security considerations](#security-considerations)

## Queues
A queue is a named list of tasks in one channel. Its tasks are sealed with a key of the channel's MLS group ([Queue key](#queue-key)), so the channel's member devices claim them ([Claim checks](#claim-checks)).

The server keeps a queue's ID, its channel and its settings, in plaintext:

| Setting | Values | Default |
|---|---|---|
| `name` | A `name` as in [Agent address](README.md#agent-address), unique in its channel | — |
| `mode` | `at-least-once` or `at-most-once`, fixed at creation: [Modes and retries](#modes-and-retries) | `at-least-once` |
| `lease` | 30 to 3600 seconds: [Claims and leases](#claims-and-leases) | 300 |
| `max_attempts` | 1 to 100; used only by `at-least-once` | 5 |

- The server issues the queue ID at creation.
- Every channel has the queue `default`, with the default settings, which the server creates when it issues the channel ID. A queue is not an agent's: a task for one agent names it as its addressee ([Tasks](#tasks)).
- A `client` or `browser` device of a member of the channel creates a queue.
- A `client` or `browser` device of the creator's account or of an admin of the space changes a setting other than `mode`, or deletes the queue with its tasks. `default` has no creator, and nobody deletes it.
- The server refuses anything else with `not-allowed`.
- Paths and bodies of the queue requests belong to the OpenAPI spec, [api/openapi.json](../../api/openapi.json). CBOR items travel as [Encoding](README.md#encoding) says.

## Tasks
A task gives an agent work whose outcome its sender follows: one claim at a time, a lease and retries, then a result or the DLQ ([Task states](#task-states)). A message only tells agents something and ends at its handover ([Hops](delivery.md#hops)). The sender chooses which it sends; no device classifies the text. Each carries one A2A Message.

A task is a record the server schedules by and a payload only devices of the channel open.

```cddl
queued-task = {            ; what the sender uploads; the server keeps and returns it unchanged
  1 => id,                 ; task: issued by the sending device
  2 => id,                 ; queue
  3 => sched,
  4 => uint .ge 1,         ; epoch: whose queue key seals `ct`
  5 => bstr .size 32,      ; enc: the HPKE encapsulated key
  6 => bstr,               ; ct: task-plaintext, sealed
  * int => any,            ; fields of later versions
}

sched = {                  ; scheduling fields, in plaintext; keys 2 and 3 are reserved
  ? 1 => bstr .size (1..32),  ; idempotency_key
  ? 4 => tstr,             ; template: a `name`
  ? 5 => timestamp,        ; deadline
  ? 6 => id,               ; device: the addressee's device
  * int => any,
}

task-plaintext = signed<task-body>   ; label "skald task", by `sender`

task-body = {
  1 => id,                 ; sender: the sending device
  2 => id,                 ; channel
  3 => id,                 ; queue
  4 => id,                 ; task
  5 => sched,              ; the record's `sched`
  6 => queue-mode,         ; the queue's mode
  7 => timestamp,          ; enqueued, as the sender states it
  8 => bstr .cbor content, ; with the task body
  * int => any,
}

queue-mode = &(at-least-once: 1, at-most-once: 2)

$$body //= (11 => task)

task = {
  1 => tstr,               ; request: one A2A Message in JSON
  * int => any,
}
```

- `task`: unique per sending device; a retried upload keeps it.
- `sched`: what the server schedules by; `task-body` repeats it, so a change by the server fails [Claim checks](#claim-checks).
  - `idempotency_key`: the server refuses a task whose key is that of another task the queue keeps, with `duplicate`. The server sees it, so it holds no secret.
  - `template`: [Templates](#templates).
  - `deadline`: the server hands out no task past it. When its user sets none, a sending device sets it 7 days after enqueue if a device of another account may claim the task.
  - `device`: the server hands the task out only to this device ([Claims and leases](#claims-and-leases)).
- The content: `from` is the enqueuing agent; `to`, exactly when `sched` has `device`, is the addressee agent, which runs on that device; `hops` and `grant` as for a message; `attachments` as for the `a2a` body. It has no `counter`: tasks are not mailbox messages, so [gap detection](delivery.md#gap-detection) does not apply.
- `request`: the A2A Message the agent works on, in the JSON form of the A2A version the protocol version pins ([A2A extension](README.md#a2a-extension)). Its `messageId` is `task` in text form, its `role` `ROLE_USER`.
- A `task` body appears only in a task. A recipient drops a message whose content holds one.

**Enqueue.** The sending device uploads a `queued-task` over HTTPS for the queue.
- It first warns its user when no device serves the queue: no [presence](agents.md#presence) value it accepted in the channel, from the task's `device` if it has one, lists the queue with a tier other than `off` for the trust level the task's sender has on that device.
- A retry carries the same bytes. The server answers a repeated upload of a `task` from the same device as it answered the first, whatever its bytes.
- The server refuses:
  - with `not-a-member`, a device that may not send in the channel ([Send](delivery.md#send)), or a `service` device whose token does not allow the queue;
  - with `duplicate`, a `task` it has from another device;
  - with `epoch-mismatch`, an `epoch` other than the channel's current or previous one;
  - with `too-large`, a `ct` over the message text limit (cloud: [architecture.md, "Cloud limits"](../dev/architecture.md#cloud-limits)); files travel as [attachments](delivery.md#attachments).
- The server keeps which device uploaded each task.

## Queue key
Each epoch of a channel's group has a queue key pair, an HPKE key pair of DHKEM(X25519, HKDF-SHA256):

- `(sk, pk) = DeriveKeyPair(ikm)` [[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180), §7.1.3], where `ikm = HKDF-SHA256(salt = "", IKM = the epoch's history key, info = "skald queue", L = 32)` [[RFC 5869](https://www.rfc-editor.org/rfc/rfc5869)].
- So every device that holds the epoch's [history key](membership.md#history-key) holds `sk`: member devices of the epoch, and devices that reach the key through the [key chain](membership.md#key-chain) or a [wrap](membership.md#wraps). Who reads which epoch: [Access](membership.md#access).
- `sk` does not give the history key, so a device that holds only `pk` reads neither tasks nor history.

**Sealing.** `enc` and `ct` are HPKE in base mode, single-shot, with the suite of [Scheme `hpke`](README.md#scheme-hpke), to the `pk` of `epoch`. `info` is the ASCII string `skald task payload`; `aad` the encoded `[channel, queue, task, epoch]`, IDs as byte strings and `epoch` an unsigned integer.

A device seals a task only as follows:
- a member device, to the `pk` of its current epoch, an epoch it judged [clean](membership.md#clean-epochs); after a browser's revocation it first commits the empty Commit, as before it sends ([History key](membership.md#history-key));
- a `browser` device, to the `pk` it derives from a wrap of the epoch it may write entries in, as [Wraps](membership.md#wraps) says;
- a `service` device that is not a member, to the `key` of a [queue key object](#queue-key-object) whose `channel` and `epoch` are those of the channel's latest GroupInfo. It MUST NOT seal unless that GroupInfo passes the checks a browser makes before it writes ([Wraps](membership.md#wraps)), it lists the object's `signer` as a member, and the object's signature verifies with the certificate of `signer`, a valid device ([Device checks](membership.md#device-checks)).

Other devices seal no task.

### Queue key object
Member devices publish each epoch's `pk` for `service` devices outside the group.

```cddl
queue-key = signed<queue-key-body>   ; label "skald queue key", by `signer`

queue-key-body = {
  1 => id,                 ; channel
  2 => uint .ge 1,         ; epoch
  3 => bstr .size 32,      ; key: the epoch's `pk`
  4 => id,                 ; signer: a member device of the epoch
  * int => any,
}
```

- The sender of each Commit uploads the queue key object of the new epoch with its link and wraps; any member device of an epoch uploads it if the server has none. A device uploads none for an epoch it did not judge clean.
- The server keeps the first per channel and epoch, and takes uploads only from member devices of the epoch.
- A member device that finds the server's object with a `key` other than the `pk` it derives MUST warn its user.
- A `service` device that is not a member holds no `sk`: it enqueues tasks and reads none.

## Task states

| State | The task |
|---|---|
| `queued` | Waits for a claim; after a failed attempt, not before its backoff ends |
| `claimed` | Held by one device until its lease ends |
| `done` | Completed by its last claim |
| `dead` | In the queue's DLQ: failed for good, released 5 times by any devices, or past its deadline |
| `cancelled` | Cancelled by a human |

| From | Event | To |
|---|---|---|
| — | Enqueue | `queued`, attempts 0 |
| `queued` | Claim | `claimed`; generation and attempts plus one |
| `queued` | `deadline` passes | `dead` |
| `claimed` | Extend | `claimed`, with a new lease |
| `claimed` | Complete | `done` |
| `claimed` | Release, the n-th since enqueue or the last restart, n from 1 to 4 | `queued` at once; attempts minus one; not handed to the releasing device for `10 × 2^(n−1)` seconds |
| `claimed` | Release, the 5th since enqueue or the last restart | `dead` |
| `claimed` | Fail with retry, or the lease ends, under `at-least-once` with attempts below `max_attempts` | `queued` after the backoff |
| `claimed` | Any other fail, or the end of the lease | `dead` |
| `dead` | Restart | `queued`, attempts and releases 0 |
| `queued`, `claimed` or `dead` | Cancel | `cancelled` |

- Extend, complete, fail and release pass the [fencing check](#fencing) first.
- **Backoff:** after the n-th failed attempt, the task waits `min(10 × 2^(n−1), 3600)` seconds.
- **Restart and cancel:** a `client` or `browser` device of the account that uploaded the task or of an admin of the space restarts a `dead` task or cancels a task; the server refuses others with `not-allowed`, a restart of a task in another state with `not-dead`, and a cancel of a `done` or `cancelled` task with `not-cancellable`. The holder of a cancelled claim gets `claim-lost`.
- The server gives a task's `queued-task`, state, attempts and its last claim's `claim_id`, `generation` and device only to devices that may send in the channel.
- The server deletes a `done`, `dead` or `cancelled` task once the channel's history retention has passed since it got there, and every task of a deleted queue.

## Claims and leases
A device claims a task from a queue, stating what it takes: the trust levels of senders, the templates, and whether it takes a task without a template. The server answers with the first task, in the order it stored them, that is `queued`, past its backoff and before its `deadline`, not in the claimer's wait after its release ([Task states](#task-states)), and that matches the claim; or with none. A task matches when:
- its `device`, if any, is the claimer;
- its uploader's trust level for the claimer, as [Trust levels](membership.md#trust-levels) sets it from the certificates and the roster the server holds, is a stated one;
- its `template` is a stated one, or it has none and the claim takes such tasks.

```cddl
claim = {
  1 => id,                 ; claim_id: issued by the server for this claim
  2 => uint .ge 1,         ; generation
  3 => uint .ge 1,         ; attempt: claims since enqueue or the last restart, released ones not counted
  4 => uint,               ; lease: seconds
  5 => queue-mode,         ; the queue's mode
  6 => id,                 ; uploader: the device that uploaded the task
  7 => queued-task,
  * int => any,
}
```

- `generation`: one more than the task's previous claim; never reused, restarts included. An agent passes it as a fencing token to systems that take one, so they refuse a write with a lower one. It uses the task ID to make its own side effects idempotent.
- **Lease:** the claim holds the task for `lease` seconds from the claim or the last extension. The holder counts it on its own clock from when it sent the request; if it passes without an answered extension, the holder SHOULD stop the work, as the server may hand the task to another device.
- **Heartbeat:** the holder sends `extend` each time a third of the lease has passed. It sends it only while the task runs or waits for its user's approval ([Running a task](#running-a-task)):
  - with the helper, the helper sends it while the agent session that holds the claim runs, or the task waits;
  - in one-shot mode without the helper, the process running the task sends it;
  - a holder whose process stops sends none, so the lease ends.
- Extend, complete, fail and release name the task, `claim_id` and `generation`; fail also states whether to retry.
- **Release:** a `client` holder hands back a task it did not start, with no result. It MUST NOT release a task it started. The server refuses a release from a `service` device with `not-allowed`; such a device fails the task instead ([Claim checks](#claim-checks)).

### Fencing
The server takes an extend, complete, fail or release only if, checked and applied in one transaction:
1. the task is `claimed`;
2. its current claim has the request's `claim_id` and `generation`;
3. the requesting device holds that claim;
4. the lease has not ended.

Otherwise it answers `claim-lost`. A repeated complete, fail or release that the server took gets the answer of the first.

On `claim-lost` the holder stops the work and sends no result.

## Modes and retries
| | `at-least-once` | `at-most-once` |
|---|---|---|
| For | Idempotent work, as the default | Work that must not repeat: a deploy, sending mail |
| A fail with retry, or the end of the lease | Back to `queued` after the backoff, until `max_attempts`; then `dead` | `dead` |
| A fail without retry | `dead` | `dead` |
| A release | Back to `queued` at once, the attempt not counted, kept from that device for a wait by the release count ([Task states](#task-states)); the 5th release since enqueue or the last restart: `dead` | The same, so the next claim has `attempt` 1 |
| After `dead` | A human restarts it | A human restarts it |

- Under `at-least-once`, a task can run more than once, also at the same time on two devices after a lease ends: its handler must be idempotent.
- Under `at-most-once`, an honest server hands out a claim with `attempt` 1 only.

## Claim checks
The server hands out a task, and takes an extend, complete, fail or release of its claim, only from a device that at that moment:
1. is a member device of the channel's current epoch, as it knows from the Commits it accepted ([Commit ordering](membership.md#commit-ordering));
2. is of kind `client` or `service`;
3. is not revoked by the revocations it stores ([Revocation](membership.md#revocation));
4. if it is a `service` device, holds a token that allows the queue.

Otherwise it answers `not-a-member`. So a `browser`, a `service` device that is not a member and a device outside the channel claim no task.

The claiming device MUST NOT run a task unless, when it starts it:
1. `ct` opens with the `sk` of `epoch`, with `aad` from the queue's channel, the claimed queue, `task` and `epoch`;
2. `signature` verifies with the certificate of `sender`, a valid device ([Device checks](membership.md#device-checks)) of kind `client`, `browser` or `service`, and `sender` is the claim's `uploader`;
3. `channel`, `queue` and `task` are those of the claim; `sched` is the same map as the record's; `mode` is the claim's;
4. the content passes checks 2 and 3 of [Envelope](README.md#envelope) with `sender` as the sending device; it has a `task` body and no `counter`, and `to` exactly when `sched` has `device`; the `request`'s `messageId` is `task` in text form;
5. `deadline`, if any, has not passed, and under `at-most-once` the claim's `attempt` is 1;
6. the task matches the claim, as [Claims and leases](#claims-and-leases) says, with the [trust level](membership.md#trust-levels) of `sender`; and `to`, if any, names an agent of this device.

- A task that fails check 1 because the device lacks the epoch's key, fails check 6, or needs a mode the device cannot enforce ([Running a task](#running-a-task)): a `client` device releases it, a `service` device fails it with retry.
- A task that fails any other check: the device fails it without retry and tells its user.

## Running a task
A device runs a task that passes the [claim checks](#claim-checks) in one of two places:
- **Executor:** a new agent session with no person, one per task: a clean context and environment, the queue's folder as its directory, and none of the user's MCP servers, hooks and plugins.
- **Working session:** a session the device's user has open, with that session's rights and context. It gets a task only as a [signal](delivery.md#device-to-agent) that a task waits; its agent then claims the task and completes it through the device. With no such session open, the device claims no task for it, so the task waits in the queue.

The device hands the agent the `request` with its `from` and the trust level of `sender`.

**Settings.** Per queue, kept only in the client's local state on the device, not on the server or in a file of the project. Only the device's user changes them, on the device.

| Setting | Values | Default |
|---|---|---|
| Folder | The executor's directory | — |
| `own` tasks | Taken or not | Asked when the device is connected |
| The tier of `peer` tasks, and of `service` tasks | `off`, `read-only`, `approve` or `full` | `off` |
| Where `peer` and `service` tasks run | The executor; at `approve` and `full`, the working session instead | The executor |
| Templates `peer` and `service` tasks may name | The built-in ones, and the team templates the user enables | The built-in ones |
| `peer` and `service` tasks without a template | Taken or not | Not taken |

| Tier | A task of the level |
|---|---|
| `off` | Is not taken |
| `read-only` | Runs at once |
| `approve` | Runs after the user's yes, with full rights |
| `full` | Runs at once |

**Rights.**
- An `own` task runs with full rights in an executor, or with the session's rights in a working session.
- A task that runs at once gets the stricter of its tier and the [policy](membership.md#policy)'s ceiling for its sender's level: full rights only where both are `full`.
- A working session gives a task the session's rights, so it takes a `peer` or `service` task that runs at once only where the policy's ceiling for its level is `full`.
- The device puts in the executor's folder the files the task hands over, such as the material of a `review`.
- A `test` task from a `peer` sender runs only at `approve` or `full`.
- The agent CLI enforces the mode; Skald adds no sandbox of its own. A mode the agent cannot enforce is refused, not weakened.

**Approval.**
- At `approve`, the device claims the task, asks its user and keeps the claim meanwhile. It hands the task to the executor or the working session only after a yes.
- A task that needs more rights than it runs with stops at once, with the reason "needs more rights". The device keeps the claim and asks its user, as at `approve`.
- After a no, or once `deadline` passes, the device fails the task without retry.

## Results
After the server takes its complete, the claiming device sends the result as an application message in the channel; after the server takes its fail, it SHOULD send one. Its content has `from` the claiming agent, which is the task's `to` if it has one; `to` the task's `from`; `hops` the task's plus one; the `task-result` body; and `attachments` as for the `a2a` body.

```cddl
$$body //= (12 => task-result)

task-result = {
  1 => id,                 ; task
  2 => id,                 ; claim_id
  3 => uint .ge 1,         ; generation
  4 => outcome,
  5 => tstr,               ; response: one A2A Message in JSON
  * int => any,
}

outcome = &(done: 1, failed: 2)
```

- `response`: the answer, or why the attempt failed; its `messageId` is the message's `msg_id` in text form ([Envelope](README.md#envelope)), its `role` `ROLE_AGENT`.
- It goes under `mls` to the channel if the task's `sender` is a member device of the claiming device's current epoch; otherwise under `hpke` to `sender`.
- A device that crashes after its complete repeats it and then sends the result: [Fencing](#fencing).
- It is handed over to the `to` agent, with receipts, as an `a2a` message is: [Device to agent](delivery.md#device-to-agent).

The task's sending device MUST drop a result unless:
1. it passes the checks of [Envelope](README.md#envelope), and its sending device is [admitted](membership.md#admitted-devices) to the channel;
2. `task` is a task this device enqueued in the channel, `to` is that task's `from`, and `from` is that task's `to` if it has one;
3. for `done`, the server gives the task as `done` with this `claim_id` and `generation`, claimed by the sending device.

It takes the first result with `done` that passes as the task's result.

## Templates
A template sets the agent's instructions and the formats of a task's input and result. A task needs none: an `own` sender usually sends free text.

- A task names its template in `sched`; a claim gets it only if it states that template ([Claims and leases](#claims-and-leases)).
- The input travels in `request` as one part with `data`, the input object in the template's format; the result in `response` the same way. Other parts, such as text and files, travel beside them.
- A claiming device takes a template's instructions and formats only from a template built into its client, `review`, `test` or `ask`, or from a team template signed by an [admin's device](membership.md#roster) of the space, once its user enables it for the queue ([Running a task](#running-a-task)). It MUST NOT take them from the server unsigned. Team templates come in a later revision of this document; adding them needs no new protocol version.
- A template grants no rights: [Running a task](#running-a-task).

## Problems
Queue requests are refused with a problem details object, as in [Server answers](README.md#server-answers).

| Problem | The request |
|---|---|
| `not-a-member` | Comes from a device that may not enqueue or claim in the queue: [Tasks](#tasks), [Claim checks](#claim-checks) |
| `not-allowed` | Creates, changes or deletes a queue, restarts or cancels a task, or releases a claim, from a device that may not; deletes `default` |
| `unknown-queue` | Names a queue the channel does not have |
| `unknown-task` | Names a task the queue does not have |
| `duplicate` | Uploads a task whose ID another device used, or whose `idempotency_key` another task of the queue has; the extension member `task` names that task |
| `epoch-mismatch` | Uploads a task sealed under an epoch other than the channel's current or previous one |
| `claim-lost` | Extends, completes, fails or releases a claim that fails the [fencing check](#fencing) |
| `not-dead` | Restarts a task that is not `dead` |
| `not-cancellable` | Cancels a task that is `done` or `cancelled` |
| `too-large` | Uploads a task over the limit |
| `quota-exceeded` | Is over a limit of its space (cloud: [architecture.md, "Cloud limits"](../dev/architecture.md#cloud-limits)) |

## Security considerations
- Scheduling is the server's word. It can withhold, delay or reorder tasks, mark them `done`, `dead` or `cancelled`, restart them and hand one task to two devices at once, an `at-most-once` one included. `at-most-once` holds only with an honest server; a claiming device only refuses a claim with `attempt` above 1.
- The match of a claim is the server's word too: it can keep a task from its addressee, or hand a device a task of a trust level, template or addressee its claim did not state, which the device does not run (check 6 of [Claim checks](#claim-checks)).
- Besides the server, `at-most-once` relies on `client` devices: one that releases a task it started, or whoever holds its credentials to the server, gets the task run again, as a restart does. A `service` device releases none.
- The server cannot read, forge or alter a task: the payload is sealed and signed, `aad` and `task-body` bind it to its channel, queue and ID, and `task-body` repeats `sched`. It can hand an old task out again, as it can hand one out twice, until its `deadline`.
- A member device cannot replay another device's task as a new one: the claim's `uploader` must be its `sender`.
- A task runs only while its sender is valid and may send in the channel: the tasks of a revoked device or a removed member stay unrun. A stolen device enqueues tasks at its account's `own` level, which run with full rights, until it is revoked.
- What a device runs from other accounts is set only on the device: neither the server nor an admin changes it, and the [policy](membership.md#policy) only lowers it for tasks that run at once. A task its user approves runs with full rights, whatever the policy.
- A revoked device keeps the queue keys of the epochs it held, as it keeps their history keys ([threat model, "Trade-offs"](../threat-model.md#trade-offs)): it reads the tasks sealed under them if it gets them. It reads none sealed after the Commit that removes it, except by a device outside the group that the server keeps from that revocation ([membership.md](membership.md#security-considerations)). It claims nothing, and its results are dropped.
- A queue key comes from the history key, so whoever reads an epoch's history reads its tasks: a member without history access is kept from earlier tasks as from earlier history, by the server.
- A `service` device that is not a member holds only queue public keys: a leaked service key with its token enqueues tasks at the `service` trust level and reads only what is sent to it under `hpke`. A leaked token alone signs no task, and it creates, changes or deletes no queue and releases, restarts or cancels no task.
- A member `service` device's key and token usually sit in one CI secret. A leaked token alone claims tasks it cannot open, and fails them or lets their leases end, the device's live claims included: they go to `dead` or back after the backoff. A leak of the whole secret also opens and runs the tasks it claims, reads what the device's keys reach and enqueues tasks at the `service` trust level. Neither releases, restarts or cancels a task, so neither makes an `at-most-once` task run twice.
- A member device can upload a wrong queue key object first. Tasks that non-member `service` devices seal to it are then unreadable to the other devices; member devices warn their users. It breaks availability, not secrecy from the server.
- A claiming device can return any result: it reaches the sender's agent as a message from that device's agent and trust level does ([Device to agent](delivery.md#device-to-agent)), and nothing shows that it is honest.
- Queue names and settings, `sched` with the addressee's device, what each claim states, and which device uploads, claims and completes each task, and when, are visible to the server. Everything the server sees: [threat model, "The server sees"](../threat-model.md#the-server-sees).
