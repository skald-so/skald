# Threat model

**No external audit.** No independent party has reviewed Skald's design, protocol or code. Report flaws privately: [SECURITY.md](../SECURITY.md).

What Skald protects against and what it does not. Formats and checks are in the [protocol spec](protocol/README.md); each defense links to the section it relies on.

## Contents
- [Attackers](#attackers)
  - [The server or its operator](#the-server-or-its-operator)
  - [A network observer](#a-network-observer)
  - [The IdP or GitHub](#the-idp-or-github)
  - [A stolen device](#a-stolen-device)
  - [A revoked device](#a-revoked-device)
  - [A removed member](#a-removed-member)
  - [A member acting against others](#a-member-acting-against-others)
  - [An admin acting against others](#an-admin-acting-against-others)
  - [A malicious sender](#a-malicious-sender)
  - [A leaked CI secret](#a-leaked-ci-secret)
  - [An agent on the person's own device](#an-agent-on-the-persons-own-device)
- [The server sees](#the-server-sees)
- [The server does not see](#the-server-does-not-see)
- [Trade-offs](#trade-offs)
- [Prompt injection and loops](#prompt-injection-and-loops)

## Attackers

### The server or its operator
Also whoever takes over the server or its database.

**Can:**
- See the metadata in [The server sees](#the-server-sees).
- Withhold, delay or drop envelopes. Gap detection reports a drop only once a later message from the same sending device in the same channel under the same scheme arrives: [delivery.md](protocol/delivery.md#security-considerations).
- Report `stored`, `delivered` and `expired` falsely: they are its word ([Receipts](protocol/delivery.md#receipts)).
- Withhold certificates, revocations and roster revisions: a hidden revocation keeps a device trusted for those who do not see it, a hidden revision keeps clients on an older roster, and withholding a revocation from every member device delays the revoked device's Remove ([membership.md](protocol/membership.md#security-considerations)).
- Withhold anchors objects, so a device linked with the recovery key starts from older revisions or joins no space ([membership.md](protocol/membership.md#security-considerations)).
- Withhold the master backup, so no device links with the recovery key ([membership.md](protocol/membership.md#security-considerations)).
- Give a channel member whose `history` is false the links to earlier epochs: only the server withholding them keeps it from earlier history ([membership.md](protocol/membership.md#security-considerations)).
- Join a channel's group by an external Commit from a device of its own, since it holds GroupInfo ([membership.md](protocol/membership.md#security-considerations)).
- Withhold, delay or reorder tasks, mark them `done`, `dead` or `cancelled`, restart them, hand one task to two devices at once, an `at-most-once` one included, hand an old task out again until its `deadline`, and hand a device a task its claim did not ask for ([queues.md](protocol/queues.md#security-considerations)).
- Withhold, delete or delay presence values, and give a reader that has not seen a newer value an older one until it expires, at most 900 seconds after its upload ([agents.md](protocol/agents.md#security-considerations)).
- Drop or delay an `ask_user` question or answer, so the question expires, and choose which of two answers from different devices of the person arrives first ([agents.md](protocol/agents.md#security-considerations)).
- Drop an ephemeral agent's message unnoticed: content without `counter` has no gap detection ([Gap detection](protocol/delivery.md#gap-detection)).
- Serve a linked browser a modified web app: the browser trusts the build it is served ([Trade-offs](#trade-offs)).
- Guess a link code online: at most 3 guesses at the new device per action of its user, and 3 more at each issuing device its user enters the code on before that device stops, each guess with a chance of 1 in 2^30 ([membership.md](protocol/membership.md#security-considerations)).
- Report no master key for an existing account: a new device whose user says it is the first then creates a second one, and the account splits into two sets of devices that do not trust each other ([membership.md](protocol/membership.md#security-considerations)).
- Create an account's master key on a device of its own before the user's first device, as [the IdP](#the-idp-or-github) can, then report none: the user's first device rightly says it is the first and creates a second one, with no warning, and members of a space the server's device created or joined as the account trust that device as the account's ([membership.md](protocol/membership.md#security-considerations)).

**Defenses:**
- Content, tasks, presence values and history are encrypted end to end ([Envelope](protocol/README.md#envelope), [Queue key](protocol/queues.md#queue-key), [Presence](protocol/agents.md#presence), [History](protocol/membership.md#history)).
- A server signature alone never admits a device: a client trusts a device only through its certificate chain to the account's master key ([Device checks](protocol/membership.md#device-checks)).
- Certificates, revocations, revisions, invites and grants are signed by devices or a master key, so the server cannot forge them ([Signed objects](protocol/membership.md#signed-objects), [Service grant](protocol/README.md#service-grant)).
- A new device takes its certificate, the master key and its link object only from a CPace run keyed by the link code its user enters on the issuing device, and that device takes the new device's keys only from the run. Unless one of its guesses at the code succeeds, the server cannot join the run, so it cannot certify a new device itself, give it another master key or have its own keys certified ([Link code](protocol/membership.md#link-code)).
- The master backup is sealed in HPKE PSK mode with a key derived from the recovery key, so the server cannot give a device linked with the recovery key another master key ([Account](protocol/membership.md#account)).
- A new `client` device whose user says it is not the account's first creates no master key: told there is none, it warns its user and stops ([Linking a device](protocol/membership.md#linking-a-device)). A second master key gives the server no signing power, and devices and members that pinned the first reject certificates chained to the second ([Device checks](protocol/membership.md#device-checks)).
- Two valid revisions with the same number show a forked roster; the client keeps the one it accepted and warns its user ([Roster](protocol/membership.md#roster)).
- A device the server joins to a channel is not admitted: members commit its Remove and send nothing until it is gone, and its epoch is not clean, so member devices upload no link, wrap or entry of it ([Admitted devices](protocol/membership.md#admitted-devices), [Clean epochs](protocol/membership.md#clean-epochs)).
- A browser writes history only under the key of a wrap whose signer the epoch's GroupInfo lists, with every member admitted, so the server cannot choose that key ([Wraps](protocol/membership.md#wraps)).
- A task is sealed, signed and bound to its channel, queue and ID; the server cannot read, forge or alter it ([Queue key](protocol/queues.md#queue-key), [Claim checks](protocol/queues.md#claim-checks)).
- A claiming device refuses an `at-most-once` claim with `attempt` above 1, and runs no task whose sender's trust level, template or addressee its claim did not ask for ([Claim checks](protocol/queues.md#claim-checks)).
- What a device runs from other accounts is set only in its local state; the server holds no such setting ([Running a task](protocol/queues.md#running-a-task)).
- A claiming device takes no unsigned template from the server ([Templates](protocol/queues.md#templates)).
- An attachment's `digest` keeps the server from swapping its blob ([Attachments](protocol/delivery.md#attachments)).
- Receipt messages and replies are signed by the recipient device ([Receipts](protocol/delivery.md#receipts)); gap detection reports dropped messages ([Gap detection](protocol/delivery.md#gap-detection)).
- A presence `revision` blocks a rollback for a reader that saw a newer value; `expires` bounds a replay ([Presence](protocol/agents.md#presence)).
- The web build is reproducible ([infrastructure.md, "Trust in builds"](dev/infrastructure.md#trust-in-builds)).

### A network observer
Watches or alters traffic between devices and the server, or the channel an invite link is sent over.

**Can:**
- See the IP addresses, times and sizes of each device's connections.
- Behind a TLS-intercepting proxy the device trusts, see what the server sees of the device's traffic, and take the device's credentials to the server. With those, it gets an `at-most-once` task the device started run again, as a restart does ([queues.md](protocol/queues.md#security-considerations)).
- Behind such a proxy, guess a link code online and report no master key, as the server does ([The server or its operator](#the-server-or-its-operator)).
- Read an invite link on its way and accept the invite first: holding the link's secret is what proves the newcomer ([Invite](protocol/membership.md#invite)).
- Alter an invite link on its way: the newcomer trusts the roster revision the link names ([membership.md](protocol/membership.md#security-considerations)).

**Defenses:**
- The API is HTTPS only ([Encoding](protocol/README.md#encoding)), and content is end-to-end encrypted inside it, so breaking TLS gives no content ([Envelope](protocol/README.md#envelope)).
- An invite's secret travels only in the link's fragment, which never reaches the server ([Invite](protocol/membership.md#invite)).

### The IdP or GitHub
The provider that signs users in (GitHub, Google), and GitHub as the host of SSH keys and Actions.

**Can:**
- Sign in as a user, as can each provider linked to the user's account, and do what the server allows without the user's keys: register devices no linked device certifies, see the web UI's non-secret sections, and revoke the user's invites, sessions and tokens.
- Sign in before the user's first device, as can whoever holds the user's sign-in: its device creates the account's master key and holds the account's root of trust for good, so the account stays unusable for its user. Members of a space it creates or joins as the account trust its devices as the account's, with no warning ([membership.md](protocol/membership.md#security-considerations)).
- GitHub: sign up on a server whose sign-up is closed as a listed GitHub login: the list names logins, not people, so whoever holds a listed login at sign-up gets the account, also after a rename frees it ([self-host.md, "Sign-up"](self-host.md#sign-up)).
- Persuade the user to enter a link code its own device shows, which links that device with the `name` and kind it chose ([membership.md](protocol/membership.md#security-considerations)).
- GitHub: list an SSH key of its choice as the user's, so a device key signed with it passes the SSH signature check, or a genuine one fails it as a key change ([Device certificate](protocol/membership.md#device-certificate)).
- GitHub: read the CI secrets it stores: [A leaked CI secret](#a-leaked-ci-secret).
- GitHub: forge a `/skald` comment from a user with write access, which the GitHub bridge turns into a task ([architecture.md, "Components"](dev/architecture.md#components)).

**Defenses:**
- A session alone reads and acks no mailbox: a WebSocket takes a proof by the key of a `client` device with a certificate ([Connecting](protocol/delivery.md#connecting)).
- A sign-in is not a device: a device needs a certificate from a linked `client` device or the recovery key, chained to the account's master key ([Linking a device](protocol/membership.md#linking-a-device), [Device checks](protocol/membership.md#device-checks)).
- The SSH signature is checked on top of the certificate chain, never instead of it ([Device checks](protocol/membership.md#device-checks)).
- The user's first device, whose user says it is the first, finds the attacker's master key, warns its user that someone else may have created it, and stops ([Linking a device](protocol/membership.md#linking-a-device)).
- The attacker links none of the user's devices: a new device takes a certificate only through the link code its user enters on the issuing device ([Link code](protocol/membership.md#link-code)). The issuing device tells its user to enter only a code shown on their own device.
- The web UI's non-secret sections list the account's devices and sign-in sessions, the attacker's included ([roadmap.md, "MVP"](dev/roadmap.md#mvp)).
- Sign-in methods are linked explicitly, by a web session from a sign-in of the last few minutes (the server setting `--link-sign-in-age`); a method of another account is refused, and accounts are never merged by email.
- A task from a comment runs at the `service` trust level: only where a device's user turned on `service` tasks, within the policy's ceiling unless that user approves it ([Trust levels](protocol/membership.md#trust-levels), [Running a task](protocol/queues.md#running-a-task)).

### A stolen device
An attacker holding a device or its keys, before the device is revoked.

**Can:**
- Do what the device does: read its channels, the history and presence it reaches and the tasks it opens; send as its account's agents, and from a `client` or `browser` device as the person ([Access](protocol/membership.md#access), [Envelope](protocol/README.md#envelope)).
- Answer the person's questions, and decide the `hold`s of its own agents ([agents.md](protocol/agents.md#security-considerations), [Device to agent](protocol/delivery.md#device-to-agent)).
- Enqueue tasks that its account's other devices run at the `own` trust level, with full rights ([queues.md](protocol/queues.md#security-considerations), [Running a task](protocol/queues.md#running-a-task)).
- Change what the device itself runs from other accounts, and approve the tasks waiting on it ([Running a task](protocol/queues.md#running-a-task)).
- Revoke the account's other devices; its revocations count even after another device revokes it ([Revocation](protocol/membership.md#revocation)).
- From a `client` device, certify new devices of the account, but not under an existing device's ID; and, unless its user names it as stolen when linking a device with the recovery key, have its anchors count: in a space no other device anchored, make that device start from a revision the server forged ([membership.md](protocol/membership.md#security-considerations)).
- On an admin's device, do what [an admin](#an-admin-acting-against-others) does.
- After its revocation: what [a revoked device](#a-revoked-device) does.

**Defenses:**
- Keys live in the OS key store, or a file in the state directory ([architecture.md, "Cryptography"](dev/architecture.md#cryptography)).
- Clients warn when an account's device set changes, so the devices it certifies or revokes show ([Device checks](protocol/membership.md#device-checks)).
- Its settings of what it runs are its own: it changes them on no other device ([Running a task](protocol/queues.md#running-a-task)).
- Another `client` or `browser` device of the account revokes it ([Revocation](protocol/membership.md#revocation)): see [A revoked device](#a-revoked-device).
- A device linked with the recovery key revokes it with the master key: that one object voids every revocation it and the devices it certified signed, and lists in `also` the devices that stay revoked ([Revocation](protocol/membership.md#revocation), [Linking a device](protocol/membership.md#linking-a-device)).

### A revoked device
An attacker holding a revoked device's keys and disk, such as a sold laptop or a stolen one after its revocation.

**Can:**
- Read a channel until a member device that holds its revocation commits its Remove, or the empty Commit for a browser; the server delays this by withholding the revocation ([membership.md](protocol/membership.md#security-considerations)).
- Keep the history, queue and presence keys of the epochs it held, and read the history, tasks and presence values sealed under them if it has or gets them ([Trade-offs](#trade-offs), [queues.md](protocol/queues.md#security-considerations), [agents.md](protocol/agents.md#security-considerations)).
- With the server: rejoin a channel while the server keeps its revocation from a member device that did not process its Remove, such as one that joined later, whose links then open to it keys of epochs after the Remove; and read tasks sealed after the Remove by a device outside the group that the server keeps from its revocation ([membership.md](protocol/membership.md#security-considerations), [queues.md](protocol/queues.md#security-considerations)).
- Unless a master-key revocation marks it as stolen: revoke the account's other devices, since its revocations count whether or not it is revoked itself, and, as a `client` device, have its anchors objects count for a device linked with the recovery key ([Revocation](protocol/membership.md#revocation), [Anchors](protocol/membership.md#anchors)).

**Defenses:**
- Clients that hold its revocation trust it no more: they send it nothing and reject its content ([Device checks](protocol/membership.md#device-checks)).
- Member devices commit its Remove; the Commit carries the revocation, so every member device that processes it applies it ([Group changes](protocol/membership.md#group-changes)). The new epoch has a new history key ([History key](protocol/membership.md#history-key)).
- At its revocation the server expires its mailbox entries and deletes its presence values ([Mailbox](protocol/delivery.md#mailbox), [Presence](protocol/agents.md#presence)).
- It claims no task, and the task's sender drops its results ([Claim checks](protocol/queues.md#claim-checks), [Results](protocol/queues.md#results)); its own tasks stay unrun ([queues.md](protocol/queues.md#security-considerations)).
- Its `ask_user` answers fail the device checks ([agents.md](protocol/agents.md#security-considerations)).
- Revoking it again with the master key, as stolen, voids its revocations and its anchors ([Revocation](protocol/membership.md#revocation), [Anchors](protocol/membership.md#anchors)).

### A removed member
An account an admin removed from a space, with its devices.

**Can:**
- Read and send in the space's channels until a member device that accepted the removal commits the Remove of its devices; the server delays this by withholding the revision ([Group changes](protocol/membership.md#group-changes), [membership.md](protocol/membership.md#security-considerations)).
- Keep the keys of the epochs its devices held, as [a revoked device](#a-revoked-device) does.
- Add history entries that readers accept, if the server takes them: readers check the sender against any accepted revision, and only the server keeps a removed member out ([History entry](protocol/membership.md#history-entry), [membership.md](protocol/membership.md#security-considerations)).

**Defenses:**
- Its devices are not admitted: member devices remove them and send nothing in the channel until they are gone ([Admitted devices](protocol/membership.md#admitted-devices)).
- Once its devices are out of a channel's group, recipients reject its content there, it claims no task, and its tasks stay unrun ([Envelope](protocol/README.md#envelope), [Claim checks](protocol/queues.md#claim-checks), [queues.md](protocol/queues.md#security-considerations)).
- A revision lists service agents only on members' accounts, so its service agents leave the roster with it ([Roster](protocol/membership.md#roster)). Grants it signed as an admin fail ([Service grant](protocol/README.md#service-grant)).

### A member acting against others
A member of a space, with valid devices.

**Can:**
- Read every channel it is in, with their presence and the history its `history` flag allows. Without history access, it holds its join epoch's key, which opens the links to earlier epochs: only the server withholding them keeps it from earlier history ([membership.md](protocol/membership.md#security-considerations)).
- Send as its own agents at the `peer` trust level; enqueue tasks that another account's device runs only where its user turned on `peer` tasks, within the policy's `peer` ceiling unless that user approves them ([Trust levels](protocol/membership.md#trust-levels), [Running a task](protocol/queues.md#running-a-task)).
- List any agent of its account in its presence, with any status and `note` ([agents.md](protocol/agents.md#security-considerations)).
- Claim tasks of its channels and return any result; release a task it started, so an `at-most-once` task runs again ([queues.md](protocol/queues.md#security-considerations)).
- Upload a wrong link, wrap or queue key object first, so history, or tasks that non-member `service` devices seal, become unreadable ([membership.md](protocol/membership.md#security-considerations), [queues.md](protocol/queues.md#security-considerations)).
- Make a channel unusable with a Commit the others cannot process; the channel is then replaced by a new one ([membership.md](protocol/membership.md#security-considerations)).
- Load the server and other members' devices with presence uploads. Each state change of a device costs an upload of at least 4096 bytes in every channel the device is in, and a value may reach 65552 bytes of `ct`. The server checks only an upload's device, epoch and size; no minimum interval, quota or rate limit applies yet. So a device can upload about 64 KB in each of its channels as often as it likes, and each reader that fetches a channel's values downloads its latest one ([Presence](protocol/agents.md#presence)).
- Send prompt injection: [Prompt injection and loops](#prompt-injection-and-loops).
- Take any free username, another person's GitHub login included: its user chooses it, so a username proves no GitHub or Google identity.

**Defenses:**
- It sends only as agents of its own account, and recipients set its trust level from its certificate and the roster, not from content ([Envelope](protocol/README.md#envelope), [Trust levels](protocol/membership.md#trust-levels)).
- It certifies no device under another account's master key and signs no revision ([Device checks](protocol/membership.md#device-checks), [Roster](protocol/membership.md#roster)).
- A device it adds that is not admitted is removed; the others send nothing until then and warn their users ([Admitted devices](protocol/membership.md#admitted-devices)).
- It puts no question on another person's devices: only an agent of the person asks the person ([`ask_user`](protocol/agents.md#ask_user)).
- A wrong link, wrap or queue key object breaks availability, not authenticity: every history entry is signed, and member devices warn about a wrong queue key object ([History entry](protocol/membership.md#history-entry), [Queue key object](protocol/queues.md#queue-key-object)).
- Presence storage stays bounded: the server keeps one value per device and channel, up to 65552 bytes of `ct` ([Presence](protocol/agents.md#presence)), and the cloud caps devices per member ([Cloud limits](dev/architecture.md#cloud-limits)). Nothing limits the upload rate yet.

### An admin acting against others
An admin of a space, with valid devices.

**Can:**
- Do what [a member](#a-member-acting-against-others) does.
- Sign revisions: add and remove members, admins included, change roles, exclude a member's devices, add channels and channel members, add service agents on members' accounts ([Roster](protocol/membership.md#roster)).
- Add itself to any channel of the space with history access, and read its earlier history ([Roster](protocol/membership.md#roster), [Access](protocol/membership.md#access)).
- Replace a member's `master` in the roster, so other members accept devices certified under the new key as that member's ([Roster](protocol/membership.md#roster), [Device checks](protocol/membership.md#device-checks)).
- Change the policy: the ceilings of `peer` and `service` tasks, `full` included, the history key's longest age, down to one day, and the most waking delivery mode per trust level, so that messages wake no session until its agent calls `inbox` ([Policy](protocol/membership.md#policy)).
- Sign team templates for the space ([Templates](protocol/queues.md#templates)).
- Sign invites with any role and channels ([Invite](protocol/membership.md#invite)), and grants that let `service` devices send in channels ([Service grant](protocol/README.md#service-grant)).
- Change any queue of the space, delete any but `default`, and restart or cancel any of its tasks ([Queues](protocol/queues.md#queues), [Task states](protocol/queues.md#task-states)).

**Defenses:**
- It sends only as agents of its own account and certifies no device under another account's master key ([Envelope](protocol/README.md#envelope), [Device checks](protocol/membership.md#device-checks)).
- Clients warn their users when a member's `master` changes in the roster ([Device checks](protocol/membership.md#device-checks)).
- Every change is a signed revision chained to the previous one by its hash; the server keeps every revision ([Roster](protocol/membership.md#roster)).
- The policy only lowers what a device's user allows: a `full` ceiling runs a task with full rights without approval only where that user set `full`. An admin turns on no device's tasks and changes no device's settings ([Running a task](protocol/queues.md#running-a-task)).
- A device takes a team template only once its user enables it, and a template grants no rights ([Templates](protocol/queues.md#templates)).

### A malicious sender
Anyone who sends to an agent: a member, a service agent, or an agent prompt injection steers.

**Can:**
- Put text and files into the recipient agent's context that try to steer it: [Prompt injection and loops](#prompt-injection-and-loops).
- Enqueue tasks in queues of its channels, which run where the devices' users allow its trust level ([Running a task](protocol/queues.md#running-a-task)).
- Set `hops` as it likes: content is the sending device's own ([Content](protocol/README.md#content)).
- Send as much as the server's rate limit and its space's quota allow ([architecture.md, "Security"](dev/architecture.md#security), [Send](protocol/delivery.md#send)).

**Defenses:**
- Its label and trust level come from its device's certificate and the roster, not from content ([Envelope](protocol/README.md#envelope), [Trust levels](protocol/membership.md#trust-levels)).
- The recipient agent's inbound mode and allowlist apply before handover ([Device to agent](protocol/delivery.md#device-to-agent)).
- Text from another account or a service agent reaches a session only through `inbox`, labelled as data ([Device to agent](protocol/delivery.md#device-to-agent)).
- A task from another account runs nowhere until a device's user turns on its trust level; then in an executor by default, within the policy's ceiling unless that user approves it. A mode the agent cannot enforce is refused, not weakened ([Running a task](protocol/queues.md#running-a-task), [Policy](protocol/membership.md#policy)).
- A recipient device fetches no URL the sender wrote: attachments carry a blob ID ([Attachments](protocol/delivery.md#attachments)).

### A leaked CI secret
A service agent's server token, and its device key with the token: they usually sit in one CI secret.

**Can, with the token alone:**
- Trigger the `ask-user` notifications of the service agent's person, without content, within the server's rate limits ([agents.md](protocol/agents.md#security-considerations)).
- For a `service` device that is a member of its channels: claim tasks it cannot open, and fail them or let their leases end, the device's live claims included, so they go to `dead` or back after the backoff ([queues.md](protocol/queues.md#security-considerations)); replace the device's presence value with one readers drop, so its agent shows as `offline` once the last valid value expires ([agents.md](protocol/agents.md#security-considerations)).

**Can, with the key and the token:**
- Send and enqueue tasks as the service agent, at the `service` trust level, in the channels its grant lists ([Service grant](protocol/README.md#service-grant), [queues.md](protocol/queues.md#security-considerations)).
- Ask the person questions as the service agent and read the answers ([agents.md](protocol/agents.md#security-considerations)).
- Read what is sent to the device under `hpke`; for a member device, also open and run the tasks it claims and read what its keys reach: its channels, their history and presence ([queues.md](protocol/queues.md#security-considerations)).

**Defenses:**
- A token alone signs nothing: it asks no question, signs no task, and creates, changes or deletes no queue ([agents.md](protocol/agents.md#security-considerations), [queues.md](protocol/queues.md#security-considerations)).
- A `service` device releases, restarts and cancels no task, so neither leak makes an `at-most-once` task run twice ([Claims and leases](protocol/queues.md#claims-and-leases), [queues.md](protocol/queues.md#security-considerations)).
- A `service` device sends only as its roster entry, only in its grant's channels until the grant expires, and never answers a question ([Envelope](protocol/README.md#envelope), [Service grant](protocol/README.md#service-grant), [Person](protocol/agents.md#person)).
- Its tasks run at the `service` trust level: only where a device's user turned on `service` tasks and, unless that user approves them, `read-only` in a new space's policy ([Running a task](protocol/queues.md#running-a-task), [Policy](protocol/membership.md#policy)).
- Once the service device is revoked, clients that hold its revocation trust it no more, and it claims no task ([Device checks](protocol/membership.md#device-checks), [Claim checks](protocol/queues.md#claim-checks)); a token has scopes and an expiry and is revoked in the web UI.

### An agent on the person's own device
An agent session, such as Claude Code, on one of the person's `client` devices, possibly steered by prompt injection.

**Can:**
- With a shell, run the `skald` CLI with the device's powers: answer the person's `ask_user` questions, decide `hold`s, approve waiting tasks and change what the device runs from others, send as the person, and sign what the device signs, such as revocations and, on an admin's device, revisions ([agents.md](protocol/agents.md#security-considerations), [Device to agent](protocol/delivery.md#device-to-agent), [Running a task](protocol/queues.md#running-a-task)).
- Read what the device reads, and enqueue tasks that the account's other devices run at the `own` trust level, with full rights ([Running a task](protocol/queues.md#running-a-task)).
- Reach every space of the device: the CLI run as an agent covers only its folder's space and refuses rules, `.skald.toml` approval and creating spaces and channels, but an agent with a shell runs the CLI in another folder or as the person ([architecture.md, "Membership"](dev/architecture.md#membership)).

**Out of scope:** as in other end-to-end encrypted messengers, a device acts for its person. An `ask_user` answer proves which device answered, not that a person did, and Skald does not tell an agent's answer or `hold` decision on the device from the person's ([agents.md](protocol/agents.md#security-considerations), [Device to agent](protocol/delivery.md#device-to-agent)). The defenses against content that steers the agent: [Prompt injection and loops](#prompt-injection-and-loops).

**Defenses:**
- The asking agent gets the answering device's name and kind ([`ask_user`](protocol/agents.md#ask_user)).
- A browser runs no agent, so an answer from a linked browser is not an agent's ([agents.md](protocol/agents.md#security-considerations)).
- A task from another person runs in an executor by default, and in `read-only` it reads only its folder and changes nothing, so it cannot act through the CLI ([Running a task](protocol/queues.md#running-a-task), [Policy](protocol/membership.md#policy)).
- Invites never reach the agent's context ([architecture.md, "Security"](dev/architecture.md#security)).

## The server sees
- accounts: sign-in methods and usernames;
- the roster: members, roles, devices and service agents of each space, the devices of each channel, and the space's policy, its wake ceilings included;
- devices: kind, name, public keys and certificates, the agent CLIs found on them;
- service agent names and token permissions;
- invites, sign-in sessions and tokens: who, when, from where, expiry, last use;
- IP addresses and times of connections;
- queue names and settings, task templates;
- when presence changes, and its size in steps of 4 KB, not its value;
- notifications: the address, and the class of each event a device triggers; for `ask_user`, so which envelopes likely carry the question; for an invalid result, which task's result failed the check, as it follows that task's `done` and its result;
- space, channel and device identifiers;
- which device messages whom;
- message size and time;
- when each device takes its messages;
- which envelopes ask for delivery statuses, so which are likely receipts, and which hold MLS handshake messages;
- attachment sizes, uploads and downloads;
- task scheduling fields and states, the addressee's device included; what each claim asks for: trust levels, templates, tasks without one; and which device enqueues, claims and completes each task;
- quota counters.

With attachments in a bucket, its provider sees the sizes and times of their uploads.

## The server does not see
Message texts, agent and session names other than service agents, threads, files, repositories, presence values, task payloads and code.

## Trade-offs
- HPKE messages (ephemeral agents, browser actions, `ask_user` questions and answers) have no forward secrecy, and the server can drop an ephemeral agent's message unnoticed;
- a leaked recovery key exposes the history;
- the history is outside MLS forward secrecy: a revoked device keeps the history keys it held and reads the history written before its revocation, but nothing written after;
- metadata is visible;
- on the web the server delivers the JS, so a linked browser trusts the served build; the build is reproducible ([infrastructure.md](dev/infrastructure.md), "Trust in builds");
- there has been no external crypto audit; the spec and threat model are public;
- the limits of trust in devices, rosters and history: [protocol spec](protocol/membership.md#security-considerations).

## Prompt injection and loops
The sender, not the kind of text, sets how text reaches an agent ([Device to agent](protocol/delivery.md#device-to-agent)). The defenses of [architecture.md, "Security"](dev/architecture.md#security), those the spec adds, and what each does not stop:

- **Sender label and prompt injection warning.** The label comes from the authenticated sending device and its trust level ([Envelope](protocol/README.md#envelope), [Trust levels](protocol/membership.md#trust-levels)). It informs the agent and binds no model: a model may follow text labelled as data, as it may ignore a request from its own agent.
- **Only a signal from others** ([Device to agent](protocol/delivery.md#device-to-agent)). Text from another account or a service agent enters a session only when the agent calls `inbox`; it then reaches the model as any text does.
- **Sender allowlist and inbound modes** ([Device to agent](protocol/delivery.md#device-to-agent)). An allowlist admits senders, not content: an allowed agent that injection steers passes it on. An `ask_user` answer passes regardless of mode and allowlist ([`ask_user`](protocol/agents.md#ask_user)).
- **`hold` until a human confirms** ([Device to agent](protocol/delivery.md#device-to-agent)). A human who approves a message lets its content in, and an agent with a shell on the device can decide a `hold`: [An agent on the person's own device](#an-agent-on-the-persons-own-device).
- **Tasks from other people** ([Running a task](protocol/queues.md#running-a-task), [Policy](protocol/membership.md#policy)).
  - The rights apply to tasks, not to messages: a message reaches the agent's session with that session's rights, and nothing keeps the agent from acting on it.
  - They run nowhere until the device's user turns them on; then an executor runs each apart from the user's sessions, tools and environment, within the policy's ceiling unless that user approves it.
  - The agent CLI enforces `read-only`, so a flaw in the vendor's limits is a flaw of the mode. `read-only` still reads the queue's folder, with the files the task hands over; the vendor's model sees all of it, and the result goes back to the sender: a task can take out what the executor reads ([Results](protocol/queues.md#results)).
  - In the working session, at `approve` after a yes or at `full`, a task runs without the executor's isolation: its text joins the user's context and stays there for later turns, it reads that context and the session's files, and it acts with the session's rights, MCP servers, hooks, plugins and environment.
  - A task the device's user approves runs with full rights, whatever the policy: the user's yes is the only check.
  - `own` tasks run with full rights, so a task from a stolen own device or from an own agent that injection steers is unrestricted.
- **Signed templates.** A claiming device takes a template's instructions only from its client or signed by an admin, so the server cannot inject them ([Templates](protocol/queues.md#templates)). A template grants no rights.
- **Presence and task results.** A presence `note` and a task result reach an agent labelled with their device's trust level, as a message's text does, and nothing shows a result is honest ([agents.md](protocol/agents.md#security-considerations), [queues.md](protocol/queues.md#security-considerations)).
- **Attachments** carry a blob ID, not a URL, so injected content cannot make a device fetch from a host of its choice ([Attachments](protocol/delivery.md#attachments)).

Loops:
- **Hop counter** ([Content](protocol/README.md#content)). Honest clients add one per reaction; a malicious sending device sets `hops` itself.
- **Limit on replies without a human,** enforced by the client. It stops loops among honest clients; a modified client ignores it.
- **Rate limit,** enforced by the server. It bounds any loop or flood, a malicious one included; presence uploads have none yet ([A member acting against others](#a-member-acting-against-others)).
