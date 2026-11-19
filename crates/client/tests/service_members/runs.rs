//! What a run with the agent's secrets takes: only `skald send` and `skald recv`, as no agent, and
//! only with both secrets.

use crate::member::{Bot, Runner};
use crate::service_kit::{KEY_VARIABLE, TOKEN_VARIABLE};
use crate::world::{ACME, run};

/// With both secrets, `skald channels`, `skald recv --agent` and `skald send --agent` fail, saying
/// the run sends and receives only. With the key secret alone, `skald recv` fails, naming the token's
/// variable; with the token alone, naming the key secret's. None writes the state directory.
#[test]
fn a_run_with_the_agents_secrets_only_sends_and_receives_with_both() {
    let bot = Bot::created(
        [3_135_740_001, 3_135_740_002],
        ["ada-member-runs-standin", "bob-member-runs-standin"],
    );
    let channel = bot.channel().to_string();
    for args in [
        vec!["channels", ACME],
        vec!["recv", "--agent", "other"],
        vec![
            "send",
            "--agent",
            "other",
            "--all",
            "--channel",
            &channel,
            "a line",
        ],
    ] {
        let refused = run(&bot.runner.machine, &args);
        assert!(!refused.success, "{args:?}: {refused:?}");
        assert!(
            refused.stderr.contains("only sends and receives"),
            "{args:?}: {refused:?}"
        );
    }
    let key = Runner::with(&[(KEY_VARIABLE, bot.secrets.key.as_str())]);
    let token = Runner::with(&[(TOKEN_VARIABLE, bot.secrets.token.as_str())]);
    for (runner, missing) in [(&key, TOKEN_VARIABLE), (&token, KEY_VARIABLE)] {
        let refused = runner.recv();
        assert!(!refused.success, "without {missing}: {refused:?}");
        assert!(
            refused.stderr.contains(missing),
            "without {missing}: {refused:?}"
        );
    }
    for runner in [&bot.runner, &key, &token] {
        assert_eq!(runner.files(), Vec::<std::path::PathBuf>::new());
    }
}
