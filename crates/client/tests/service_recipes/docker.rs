//! The Docker example: a long-lived member agent in a container of an image on `debian:13-slim`,
//! its state directory on a volume, that exchanges messages with Ada's device and keeps doing so
//! after its container is recreated on the same volume. On Linux, whose containers share the
//! machine's network and run the `skald` these tests build.

#[cfg(target_os = "linux")]
mod run {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use crate::common::CLIENT;
    use crate::machine::Machine;
    use crate::member::MEMBER;
    use crate::page::{
        DOCKER, DOCKERFILE, Recipe, block, install_command, install_instruction, with_channel,
        with_names,
    };

    /// What Ada says once the agent has been in the channel, before its container is recreated;
    /// nowhere else.
    const ADA_SAYS: &str = "recipe test: ada asks the recreated container for the rollout";

    /// The GitHub stand-in's users of the test: Ada and Bob.
    const IDS: [u64; 2] = [3_510_902_001, 3_510_902_002];
    const LOGINS: [&str; 2] = ["ada-docker-recipe-standin", "bob-docker-recipe-standin"];

    /// The instruction that takes the place of the Dockerfile's install instruction.
    const INSTALL: &str = "COPY skald /usr/local/bin/skald";

    /// How long a run of the docker block may take, its image's build and pull included, and how
    /// long the test waits after it for the agent's exchange.
    const BLOCK_LIMIT: Duration = Duration::from_secs(600);
    const EXCHANGE_LIMIT: Duration = Duration::from_secs(120);

    /// The settings of Docker's command line the block runs with, besides `PATH` and `HOME`.
    const DOCKER_SETTINGS: [&str; 4] = [
        "DOCKER_HOST",
        "DOCKER_CONFIG",
        "DOCKER_CONTEXT",
        "XDG_RUNTIME_DIR",
    ];

    /// The Docker objects of one test, by the name that takes the place of `deploy-bot`: its
    /// image, the containers of that image and the volumes whose names start with it, removed on
    /// drop by their exact names.
    struct Objects {
        name: String,
    }

    impl Objects {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("a clock after 1970")
                .subsec_nanos();
            Objects {
                name: format!("skald-recipe-test-{}-{nanos}", std::process::id()),
            }
        }

        /// The IDs of the containers of the image.
        fn containers(&self) -> Vec<String> {
            let ancestor = format!("ancestor={}", self.name);
            lines(&["ps", "--all", "--quiet", "--filter", &ancestor])
        }

        /// What the containers of the image printed, each container's stdout and stderr.
        fn logs(&self) -> String {
            let mut logs = String::new();
            for id in self.containers() {
                if let Ok(out) = Command::new("docker").args(["logs", &id]).output() {
                    logs.push_str(&String::from_utf8_lossy(&out.stdout));
                    logs.push_str(&String::from_utf8_lossy(&out.stderr));
                }
            }
            logs
        }

        /// Removes the containers of the image.
        fn remove_containers(&self) {
            for id in self.containers() {
                docker_quietly(&["rm", "--force", &id]);
            }
        }
    }

    impl Drop for Objects {
        fn drop(&mut self) {
            self.remove_containers();
            let images = lines(&["image", "ls", "--quiet", &self.name]);
            if !images.is_empty() {
                docker_quietly(&["image", "rm", "--force", &self.name]);
            }
            let filter = format!("name={}", self.name);
            for volume in lines(&["volume", "ls", "--quiet", "--filter", &filter]) {
                if volume.starts_with(&self.name) {
                    docker_quietly(&["volume", "rm", "--force", &volume]);
                }
            }
        }
    }

    /// The lines `docker <args>` prints; none if it fails.
    fn lines(args: &[&str]) -> Vec<String> {
        Command::new("docker")
            .args(args)
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::to_owned)
                    .filter(|line| !line.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Runs `docker <args>`, its outputs dropped.
    fn docker_quietly(args: &[&str]) {
        let _ = Command::new("docker")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    /// The Dockerfile with its install instruction replaced by [`INSTALL`]; fails unless a `RUN`
    /// instruction holds README's install command.
    #[track_caller]
    fn dockerfile_of(dockerfile: &str, channel: &str) -> String {
        let range = install_instruction(dockerfile, &install_command()).unwrap_or_else(|| {
            panic!("no RUN instruction of the Dockerfile installs skald by README's command:\n{dockerfile}")
        });
        let mut lines: Vec<&str> = dockerfile.lines().collect();
        lines.splice(range, [INSTALL]);
        with_channel(&(lines.join("\n") + "\n"), channel)
    }

    /// The docker block with the substitutions of the tests.
    fn commands_of(commands: &str, channel: &str, name: &str) -> String {
        let commands = with_names(&with_channel(commands, channel), name);
        commands
            .replace("docker build", "docker build --network none")
            .replace("docker run", "docker run --network host")
    }

    /// The directory of the build: the Dockerfile and this commit's `skald`, under a machine's of
    /// the tests.
    fn build_dir(machine: &Machine, dockerfile: &str) -> PathBuf {
        let dir = machine.root().join("build");
        fs::create_dir(&dir).expect("create the build's directory");
        fs::write(dir.join("Dockerfile"), dockerfile).expect("write the Dockerfile");
        fs::copy(CLIENT, dir.join("skald")).expect("copy skald into the build's directory");
        dir
    }

    /// What a run of the block printed, and whether it succeeded.
    #[derive(Debug)]
    struct Ran {
        success: bool,
        output: String,
    }

    /// Runs `commands` with bash in `dir`, with only `secrets`, `PATH`, `HOME` and Docker's
    /// settings in its environment, for up to [`BLOCK_LIMIT`]; its outputs go to a file in
    /// `machine`'s directory.
    fn run_block(
        machine: &Machine,
        dir: &Path,
        commands: &str,
        secrets: &[(&'static str, String)],
    ) -> Ran {
        let (script, output) = (
            machine.root().join("docker.sh"),
            machine.root().join("output"),
        );
        fs::write(&script, commands).expect("write the docker block");
        let out = fs::File::create(&output).expect("create the block's output");
        let mut command = Command::new("bash");
        command
            .args(["-eo", "pipefail"])
            .arg(&script)
            .current_dir(dir)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(out.try_clone().expect("share the block's output"))
            .stderr(out);
        for name in ["PATH", "HOME"].iter().chain(DOCKER_SETTINGS.iter()) {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.envs(secrets.iter().map(|(name, value)| (name, value)));
        let mut child = command.spawn().expect("run bash");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("wait for the block") {
                break status;
            }
            if started.elapsed() > BLOCK_LIMIT {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "the docker block ran for more than {BLOCK_LIMIT:?}:\n{}",
                    fs::read_to_string(&output).unwrap_or_default()
                );
            }
            thread::sleep(Duration::from_millis(200));
        };
        Ran {
            success: status.success(),
            output: fs::read_to_string(&output).unwrap_or_default(),
        }
    }

    /// Waits up to [`EXCHANGE_LIMIT`] until Ada's runs show a message from the agent and, given
    /// `ada_says`, what the block's run and the image's containers printed holds it; gives those
    /// messages, each sent by the agent's device at the `service` level. Fails otherwise.
    #[track_caller]
    fn exchanged(recipe: &Recipe, objects: &Objects, ran: &Ran, ada_says: Option<&str>) {
        let started = Instant::now();
        let mut said = Vec::new();
        let mut ada_runs = Vec::new();
        loop {
            let printed = format!("{}\n{}", ran.output, objects.logs());
            if said.is_empty() {
                let (from_agent, got) = recipe.ada_reads_from_the_agent();
                said = from_agent;
                ada_runs.push(got);
            }
            let shown = ada_says.is_none_or(|text| printed.contains(text));
            if shown && !said.is_empty() {
                break;
            }
            assert!(
                started.elapsed() < EXCHANGE_LIMIT,
                "after {EXCHANGE_LIMIT:?}, the agent showed Ada's line {ada_says:?}: {shown}; \
                 Ada's runs showed its messages: {}\nwhat the agent printed:\n{printed}\n\
                 Ada's runs: {ada_runs:?}",
                !said.is_empty()
            );
            thread::sleep(Duration::from_secs(1));
        }
        for message in said {
            assert_eq!(message["device_name"], "ci/deploy-bot", "{message}");
            assert_eq!(message["level"], "service", "{message}");
        }
    }

    /// The docker block, with this commit's `skald` in the image and only the two secrets set,
    /// builds the image and runs the agent, a member of the channel: it succeeds, and Ada's device
    /// shows a message from the agent's address, sent by its device at the `service` level. Ada
    /// says a line; the image's containers are removed, and the block runs again, a new container
    /// on the same volume: it succeeds, the agent shows Ada's line, and Ada's device shows a new
    /// message from the agent.
    #[test]
    fn the_docker_example_exchanges_messages_and_again_after_its_container_is_recreated() {
        let (dockerfile, commands) = (block(DOCKERFILE), block(DOCKER));
        let recipe = Recipe::new(IDS, LOGINS, &[MEMBER]);
        let channel = recipe.channel();
        let objects = Objects::new();
        let machine = Machine::new();
        let dockerfile = dockerfile_of(&dockerfile, &channel);
        let commands = commands_of(&commands, &channel, &objects.name);
        let dir = build_dir(&machine, &dockerfile);
        let secrets = recipe.variables();

        let first = run_block(&machine, &dir, &commands, &secrets);

        assert!(
            first.success,
            "the docker block failed:\n{commands}\n{dockerfile}\n{}",
            first.output
        );
        exchanged(&recipe, &objects, &first, None);

        recipe.ada_says(ADA_SAYS);
        objects.remove_containers();
        let second = run_block(&machine, &dir, &commands, &secrets);

        assert!(
            second.success,
            "the docker block failed after its container was removed:\n{commands}\n{}",
            second.output
        );
        exchanged(&recipe, &objects, &second, Some(ADA_SAYS));
    }
}
