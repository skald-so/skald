//! Reading and searching a channel's history in the CLI (`docs/protocol/membership.md`, "Key
//! chain", "History entry" and "Access").
//!
//! - The history command shows a channel's messages from its entries, oldest first; given a text,
//!   only those whose text contains it. The search runs on the client: no request carries the text.
//! - It opens entries with the keys of the epochs the device holds and of earlier ones through every
//!   link it can open. A key from a link never lies in the clear on disk.
//! - It shows an entry only if it passes the checks of "History entry", the sender's certificate
//!   checked as "Device checks" says, revoked or not, and drops any other with a warning naming the
//!   sending device and the check. It shows each message once, and says how many entries it holds
//!   no key for.
//! - As an agent, it reads only channels of its folder's space, and labels each message with its
//!   sender's trust level, with one warning line before the first `peer` or `service` line.
//!
//! As the tests of history entries (`history_entries/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces; Bob's and Carol's devices are played by the tests with an MLS state of
//! their own. A played device says a message as its own client would: it sends the message in its
//! current epoch, then uploads the message's entry under that epoch's key, signed by its device,
//! and the sender of a played Commit uploads the new epoch's link. A compromised device writes
//! entries the checks drop; a compromised server is played by the relay, which adds entries to
//! those the server gives. The relay records every byte the machines send. Messages are sent at
//! least a second apart, so their times order them.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages and of
//! history entries:
//! - `skald history <channel> [<text>]`, the channel by its ID as `skald channels` shows it,
//!   succeeds and shows each message on a line of its own, oldest first: the line holds the
//!   channel's ID, `from`, the sending device's certificate `name`, the entry's `timestamp` in UTC
//!   as RFC 3339 to the second, such as `2026-11-17T08:43:03Z`, and the text once, its control
//!   characters escaped as `skald recv` escapes them; with `<text>`, only messages whose text holds
//!   it;
//! - it says how many entries it holds no key for on a line holding `no key` and that number;
//! - an entry it drops is a warning line, saying `warning`, that holds the sending device's ID and
//!   never the message's text; one for a body of another channel says `channel`, for a `from` that
//!   is no agent of the sender's account `from`, for an A2A `messageId` other than `msg_id`
//!   `messageId`, and for a signature that does not verify `signature`;
//! - `skald --agent <name> history <channel>` in a folder bound to a space fails for a channel of
//!   another space, naming nothing of that space, and requests nothing of its history.
//!
//! The server's interface, as these tests use it: `GET
//! /spaces/{space}/channels/{channel}/history/entries`, whose answers the relay changes.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agent;
mod dropped;
mod expired;
mod gone;
mod joiners;
mod levels;
mod proxy;
mod reading;
mod search;
mod shown;
mod unaccepted;
mod waiting;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the server's
// tests' times.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../../../server/tests/common/times.rs"]
mod times;
