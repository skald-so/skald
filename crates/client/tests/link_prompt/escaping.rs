//! The waiting device's registered `name` is the server's text: the line shows its control
//! characters and line breaks escaped, on one line.

use crate::fixtures::fronted;
use crate::kit::RECV;
use crate::machine::DEVICES;
use crate::prompt::{assert_prompted, waiting_device};
use crate::world::run;

/// The host name of Ada's new machine, its registered `name`: an escape sequence, a line break and
/// a bell inside it.
const RAW: &str = "ada-\u{1b}[31mlaptop\nskald: forged line\u{7}";
/// It as the CLI shows text from the server.
const SHOWN: &str = r"ada-\u{1b}[31mlaptop\nskald: forged line\u{7}";

/// Ada's new machine, whose host name holds an escape sequence, a line break and a bell, waits for
/// linking. Her first machine's `skald devices` and `skald recv` each show its name once, escaped,
/// on the line that says the server reports it; their output holds no escape character, line break
/// of the name or bell.
#[test]
fn a_name_with_control_characters_shows_escaped_on_one_line() {
    let (world, _front) = fronted();
    let ada = world.sign_up(
        4_178_301_001,
        "ada-with-a-strange-laptop",
        "ada-workstation",
    );
    let _new = waiting_device(&world, &ada, RAW);

    for args in [[DEVICES], [RECV]] {
        let shown = run(&ada.machine, &args);
        assert!(shown.success, "{args:?}: {shown:?}");
        let text = shown.shown();
        assert_prompted(&text, SHOWN);
        assert!(!text.contains(['\u{1b}', '\u{7}']), "{args:?}: {text:?}");
        let broken: Vec<&str> = (text.lines())
            .filter(|line| line.starts_with("skald: forged line"))
            .collect();
        assert!(broken.is_empty(), "{args:?}: {text:?}");
    }
}
