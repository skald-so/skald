# Load testing

Full runs use an isolated GitHub-hosted Ubuntu 24.04 x64 runner with 4 vCPUs, 16 GB of RAM and 14 GB of SSD storage. All scenario traffic stays on the runner. `script/test` uses short runs with small fixtures. [Runner specifications](https://docs.github.com/en/actions/reference/runners/github-hosted-runners#standard-github-hosted-runners-for-public-repositories)

## Run profile

Use the [self-host stack](../self-host.md) with its Caddy [behind a TLS proxy](../self-host.md#behind-your-reverse-proxy). Start one instance of the server image built from the tested commit as [`script/image-test`](../../script/image-test) builds it. For the two-instance scenario, have Caddy distribute connections across two identical instances. Set sign-up to open and point GitHub sign-in at the [local stand-in](../../crates/github-stand-in). Apply the server settings in [Cloud limits](architecture.md#cloud-limits), including the sliding-window send limit. Run the load tool in its own container.

| Container | CPU cap (vCPUs) | Memory cap |
|---|---:|---:|
| Skald server, per instance | 0.75 | 2 GiB |
| PostgreSQL 19 | 1 | 4 GiB |
| Stack Caddy | 0.25 | 512 MiB |
| TLS proxy | 0.25 | 512 MiB |
| GitHub stand-in | 0.125 | 256 MiB |
| Load tool | 0.625 | 2 GiB |

Set each container's memory-plus-swap limit equal to its memory cap to disable swap. The caps total 3.75 vCPUs and 11.25 GiB with two server instances. Set the open-file limit to 32,768 on the load tool, server and proxies. Each proxy needs two sockets per WebSocket. [Docker container limits](https://docs.docker.com/engine/containers/resource_constraints/#--memory-swap-details)

PostgreSQL uses `shared_buffers=1GB` and a `max_wal_size` checkpoint threshold of 1 GB. Cap its disk-backed volume for data and WAL at 6 GiB. Set `max_connections=40` to allow [11 connections per server instance](architecture.md#network), replacement listeners and monitoring. Enable `pg_stat_statements` for the run. [WAL settings](https://www.postgresql.org/docs/19/runtime-config-wal.html#GUC-MAX-WAL-SIZE) · [Statement statistics](https://www.postgresql.org/docs/19/pgstatstatements.html)

## Population

Connect 5,000 devices over WebSocket from 1,000 synthetic accounts. Use 50 shared spaces, each with one measured channel of 20 accounts. In 45 spaces each account has four devices, in four spaces each has five, and in the large channel each has 50. The large channel tests the [devices limit](architecture.md#cloud-limits). Each account belongs to one measured channel and owns at most one shared space in addition to its personal space.

| Activity | Per device per hour | Workload assumption |
|---|---:|---|
| Messages | 1 | Mostly short agent updates, with larger payloads for context. Use the largest encoded payload including MLS padding at or below 4 KiB for 90%, 16 KiB for 9%, and the 64 KiB limit for 1%. Store an encrypted history entry with each message. |
| Presence | 6 refreshes, 2 state changes, 2 channel reads | Stagger refreshes every 600 seconds to leave time before expiry. Read the channel on reconnect and while working. Refresh on entry to a new epoch as well. |
| Task submissions | 0.2 | Simulate one two-minute task per five device-hours. |
| MLS Commits initiated | 0.01 | Simulate occasional device or epoch changes, at one Commit per 100 device-hours. |
| Reconnects | 0.25 | Simulate routine connection churn. Model a deploy with a two-minute receive gap and a peak wave of 2,500 devices. |

Use four times the hourly average for peak messages, task submissions, Commits, presence state changes and reads. Keep periodic refreshes staggered. The four-hour soak combines the workloads at the population rates.

## Scenario targets

Round doubled peak rates up to whole operations per reporting interval.

| Scenario | Target | Basis |
|---|---|---|
| Delivery and history | Store and push 12 channel messages per second, each with its [history entry](../protocol/membership.md#uploads). Ack every push. | Twice the peak of 5,000 devices sending one message per hour, with the payload mix above up to the [message text limit](architecture.md#cloud-limits). Rotate senders within the device rate limit. |
| Cursor catch-up | Reconnect all 5,000 devices at once, each with at least 25 missed messages. Check mailbox order, then resume live pushes. | Twice the peak reconnect wave. During the two-minute receive gap, 26 distinct senders per channel each send once with delivery reports off, twice the 13 sends modeled at peak. [Senders receive no copy of their own messages](../protocol/delivery.md#hops). |
| Large-channel fan-out | One device sends 120 messages per rolling minute to the other 999 devices in the large channel. Measure time to the last push. | The [per-device send limit](architecture.md#cloud-limits) at 20 members with 50 devices each. |
| Large-channel MLS Commits | Accept and push 80 Commits per hour in the large channel. Exercise Adds, Removes and empty Commits with the [new epoch uploads](../protocol/membership.md#uploads). | Twice the large channel's peak of 40 Commits per hour. Submit Commits against the current epoch. |
| Presence | Store 39 presence values per second and serve 23 channel reads per second, including the large channel. Check the latest values byte for byte. | Twice the refresh and peak state-change workload of about 19.4 values per second, and twice the peak of about 11.1 reads per second. Keep each device's value current within the [900-second protocol interval](../protocol/agents.md#presence). |
| Task queue | Enqueue, claim and complete 3 tasks per second each, extend 18 leases per second, and deliver completed results to their senders. Exercise both modes. | Twice the population peak, rounded up. Split two-minute tasks evenly between [30- and 300-second leases](../protocol/queues.md#queues). Extending each third of the lease gives 11 and 1 [heartbeats](../protocol/queues.md#claims-and-leases) per task, respectively. |
| Two instances | Meet the delivery and history target with 2,500 devices per instance and senders and recipients on different instances. | Same target with cross-instance pushes through [PostgreSQL `LISTEN/NOTIFY`](architecture.md#network). |

For catch-up, send over HTTPS while recipients' WebSockets are down. In the restart variant, fill the backlog after HTTPS is ready and before releasing the reconnect wave. For tasks, also check that expired leases reach the expected queue state and that no task has two live claims.

## Step bounds

Increase the load step by step. The last passing step sets the scenario maximum.

| Measure | Bound |
|---|---:|
| p99 push, from `stored` to recipient device | 2 seconds |
| Catch-up, from wave release until the last device receives its backlog | 120 seconds |
| p99 claim, from request to successful response | 1 second |
| p99 Commit submission to `stored` | 2 seconds |
| p99 Commit submission to last recipient | 10 seconds |

A step fails on any HTTP 5xx, refusal, container kill, or lost or duplicated message, mailbox entry, presence value or task.
