//! The GitHub Actions step: a one-shot agent that installs `skald`, sends a message in the channel
//! and reads the channel's messages, with only the environment the step sets.

#[cfg(unix)]
mod run {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use crate::common::CLIENT;
    use crate::machine::{Finished, Machine, Run};
    use crate::page::{
        ACTIONS, Recipe, Step, block, compact, from_secrets, install_command, step_of, with_channel,
    };

    /// What Ada says before the step runs; nowhere else.
    const ADA_SAYS: &str = "recipe test: ada asks the ci run to report the rollout";

    /// The GitHub stand-in's users of the test: Ada and Bob.
    const IDS: [u64; 2] = [3_510_901_001, 3_510_901_002];
    const LOGINS: [&str; 2] = ["ada-actions-recipe-standin", "bob-actions-recipe-standin"];

    /// A runner's directories, under a machine's of the tests: the job's working directory, and
    /// those of its command files and of the install stand-in.
    struct Runner {
        machine: Machine,
    }

    impl Runner {
        fn new() -> Self {
            let machine = Machine::new();
            for dir in ["workspace", "runner", "stand-in"] {
                fs::create_dir(machine.root().join(dir)).expect("create a runner's directory");
            }
            Runner { machine }
        }

        fn dir(&self, name: &str) -> PathBuf {
            self.machine.root().join(name)
        }

        /// The stand-in for README's install command: puts this commit's `skald` where
        /// `install.sh` puts it, and leaves `PATH` as it was.
        fn install_stand_in(&self) -> PathBuf {
            let path = self.dir("stand-in").join("install.sh");
            let script = format!(
                "#!/bin/sh\nset -e\ndir=${{SKALD_INSTALL_DIR:-$HOME/.local/bin}}\nmkdir -p \"$dir\"\n\
                 cp '{CLIENT}' \"$dir/skald\"\nchmod 755 \"$dir/skald\"\n\
                 echo \"install.sh: installed skald in $dir\"\n"
            );
            fs::write(&path, script).expect("write the install stand-in");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("make the install stand-in executable");
            path
        }

        /// Runs `script` as GitHub Actions runs a step's script with `shell`, unset or `bash`, and
        /// the step's `env`: in the job's working directory, with a runner's environment besides.
        fn run(&self, script: &str, shell: Option<&str>, env: &[(String, String)]) -> Finished {
            let file = self.dir("runner").join("step.sh");
            fs::write(&file, script).expect("write the step's script");
            let mut command = Command::new("bash");
            match shell {
                Some(_) => command.args(["--noprofile", "--norc", "-eo", "pipefail"]),
                None => command.arg("-e"),
            };
            command
                .arg(&file)
                .current_dir(self.dir("workspace"))
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", self.dir("home"))
                .env("TMPDIR", self.dir("tmp"))
                .env("RUNNER_TEMP", self.dir("tmp"))
                .env("GITHUB_WORKSPACE", self.dir("workspace"))
                .stdin(Stdio::null());
            for name in [
                "GITHUB_ENV",
                "GITHUB_PATH",
                "GITHUB_OUTPUT",
                "GITHUB_STEP_SUMMARY",
            ] {
                let path = self.dir("runner").join(name);
                fs::write(&path, "").expect("create a runner's command file");
                command.env(name, path);
            }
            command.envs(env.iter().map(|(name, value)| (name, value)));
            Run::spawn(&mut command).finish()
        }
    }

    /// The step's script with the substitutions of the tests: the channel, and README's install
    /// command replaced by `stand_in`. Fails unless the script holds that command.
    #[track_caller]
    fn script_of(step: &Step, channel: &str, stand_in: &Path) -> String {
        let install = install_command();
        assert!(
            step.run.contains(&install),
            "the step's script does not install skald by README's command {install:?}:\n{}",
            step.run
        );
        let script = step
            .run
            .replace(&install, &format!("sh '{}'", stand_in.display()));
        with_channel(&script, channel)
    }

    /// The step's env with each `${{ secrets.<name> }}` of the two secrets replaced by its value.
    fn env_of(step: &Step, recipe: &Recipe) -> Vec<(String, String)> {
        let variables = recipe.variables();
        step.env
            .iter()
            .map(|(name, value)| {
                let value = variables
                    .iter()
                    .find(|(secret, _)| compact(value) == compact(&from_secrets(secret)))
                    .map(|(_, secret)| secret.clone())
                    .unwrap_or_else(|| value.clone());
                (name.clone(), value)
            })
            .collect()
    }

    /// Ada says a line in the channel; then the step's script runs, with this commit's `skald` in
    /// place of the released one and only the environment the step sets: it succeeds and shows
    /// Ada's line, and Ada's next run shows a message from the agent's address, sent by its
    /// device, `ci/deploy-bot`, at the `service` level.
    #[test]
    fn the_github_actions_step_exchanges_a_message_with_a_member_device() {
        let step = step_of(&block(ACTIONS));
        let recipe = Recipe::new(IDS, LOGINS, &[]);
        let runner = Runner::new();
        let script = script_of(&step, &recipe.channel(), &runner.install_stand_in());
        recipe.ada_says(ADA_SAYS);

        let ran = runner.run(&script, step.shell.as_deref(), &env_of(&step, &recipe));

        assert!(ran.success, "the step's script failed:\n{script}\n{ran:?}");
        assert!(
            ran.shown().contains(ADA_SAYS),
            "the step's run does not show Ada's line:\n{script}\n{ran:?}"
        );
        let (said, got) = recipe.ada_reads_from_the_agent();
        assert!(
            !said.is_empty(),
            "Ada's run shows no message from {}: {got:?}\nthe step's run: {ran:?}",
            recipe.address
        );
        for message in &said {
            assert_eq!(message["device_name"], "ci/deploy-bot", "{message}");
            assert_eq!(message["level"], "service", "{message}");
            assert!(
                !message["text"].as_str().unwrap_or_default().is_empty(),
                "{message}"
            );
        }
    }
}
