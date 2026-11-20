//! The recipes of the service agents page, `docs/service-agents.md`: a GitHub Actions step and a
//! Docker example that exchange messages in a channel with only the agent's two secrets set,
//! `SKALD_SERVICE_KEY` and `SKALD_SERVICE_TOKEN`, under those names; the key secret names the
//! server.
//!
//! The page has them as snippets to copy, in marked code blocks, as the self-host guide has its
//! commands: fenced with the info string `<language> service-agent=<step>`, one block per step. A
//! marked block of a step these tests do not run fails them, and so does a step without a block.
//! The steps:
//!   github-actions  a GitHub Actions step, YAML: a job's `steps:` item or its mapping, with the keys
//!                   `name`, `env`, `run` and `shell` (`bash`) only, `run` a literal block or a line.
//!                   A one-shot agent, without a state directory: its `env` sets the two secrets
//!                   from the repository's secrets of the same names, `${{ secrets.<name> }}`, and
//!                   nothing else; its script installs `skald` as README's "Install the client"
//!                   says, by its command for macOS and Linux, sends a message in the channel and
//!                   reads the channel's messages.
//!   dockerfile      the Dockerfile of the Docker example: an image `FROM debian:13-slim` pinned by
//!                   digest that installs `skald` in a `RUN` instruction holding README's command
//!                   and runs it from `PATH`.
//!   docker          shell commands, run in the directory that holds that Dockerfile as
//!                   `Dockerfile`: `docker build` builds the image, and `docker run` runs a
//!                   long-lived member agent of it (`--member`), its state directory on a volume,
//!                   with the two secrets passed from the shell's environment. The agent sends a
//!                   message in the channel and reads the channel's messages, whether its
//!                   container does so once and exits or keeps running; the tests read its output
//!                   from the block's and from the logs of the image's containers. The image and
//!                   the volume are named after the agent, `deploy-bot`.
//!
//! What the tests substitute, and nothing else:
//! - `<channel>`, in every block: the talk's channel's ID;
//! - README's install command, in the step's script: a stand-in that puts this commit's `skald`
//!   where `install.sh` puts it, `${SKALD_INSTALL_DIR:-$HOME/.local/bin}`, and, as it does, leaves
//!   `PATH` as it was;
//! - the Dockerfile's `RUN` instruction that holds README's install command, as a whole:
//!   `COPY skald /usr/local/bin/skald`, this commit's `skald` built for Linux, in the build's
//!   directory;
//! - in the docker block, `deploy-bot` where no `/` comes before it: a name of the test's own, so
//!   the image and the volume are new and the test removes them by that name; `docker build` gets
//!   `--network none` and `docker run` `--network host`, so the build reaches no host and the
//!   container reaches the local server where the key secret names it.
//!
//! With the install lines substituted, the runs reach no host but the local server and the image
//! registry the base image comes from.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): Ada's machine runs the CLI and
//! creates `acme`; Bob's device is played by the tests. Ada's machine creates `ci/deploy-bot` for
//! the talk's channel as the tests of service agents do (`service_agents/main.rs`): without
//! `--member` for the step, with it for the Docker example.
//! - The step's script runs with bash as GitHub Actions runs a step's script, with only the
//!   environment the step sets, beside a runner's own: `PATH` without `skald`, an empty home, an
//!   empty temporary directory and working directory, and the runner's empty command files.
//! - The docker block runs with bash, with only the two secrets beside what `docker` needs:
//!   `PATH`, `HOME` and Docker's own settings. A member that joins the group after a message
//!   cannot read it, so a message of Ada's reaches the agent's run after its container is
//!   recreated. On Linux only, as the containers share the machine's network.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod actions;
mod docker;
mod docs;

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The service agents' commands and secrets, as the tests of service agents have them, and what a
// run of `skald recv --json` shows, as the tests of member agents have it.
#[path = "../service_members/member.rs"]
mod member;
#[path = "../service_agents/kit.rs"]
mod service_kit;

mod page;
