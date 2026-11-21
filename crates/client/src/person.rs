//! Where a command takes its person's answers and shows what only the person may see, such as a
//! link code or a recovery key: at the person's terminal, or on the local page that `--browser`
//! opens ([`crate::page`]), never in the command's output.

use std::io::{self, BufRead, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::page::{Answer, Page};

/// Where its person answers.
#[derive(Clone, Copy)]
pub enum Person<'p> {
    /// At the terminal of the command: its standard input and output.
    Terminal,
    /// On the command's local page.
    Page(&'p Page),
}

impl Person<'_> {
    /// Shows `text`, such as a link code, to the person only: at the terminal, as a line of its
    /// output; on the page, in place of what it showed before. `None` shows nothing more.
    pub fn show(self, text: Option<&str>) -> Result<(), String> {
        match self {
            Person::Terminal => {
                if let Some(text) = text {
                    println!("{text}");
                }
                Ok(())
            }
            Person::Page(page) => page.show(text),
        }
    }

    /// Whether its person answers yes to `question`, asked again at the terminal until yes or no;
    /// `None` once the terminal's input ends.
    pub fn ask(self, question: &str) -> Result<Option<bool>, String> {
        match self {
            Person::Terminal => ask(&mut io::stdin().lock(), &mut io::stdout(), question)
                .map_err(|err| format!("cannot ask the user: {err}")),
            Person::Page(page) => {
                let answers = page.ask(question, &["Yes", "No"], None, true)?;
                Ok(answers
                    .recv()
                    .ok()
                    .map(|answer| answer == Answer::Button(0)))
            }
        }
    }

    /// The text its person enters for `label`, sent on the page by `button`, without its line's
    /// end; `None` once the terminal's input ends.
    pub fn enter(self, label: &str, button: &str) -> Result<Option<String>, String> {
        match self {
            Person::Terminal => read(
                &mut io::stdin().lock(),
                &mut io::stdout(),
                &format!("{label}: "),
            )
            .map_err(|err| format!("cannot read the answer: {err}")),
            Person::Page(page) => {
                let answers = page.ask("", &[button], Some(label), true)?;
                Ok(answers.recv().ok().map(|answer| match answer {
                    Answer::Text(text) => text,
                    Answer::Button(_) => String::new(),
                }))
            }
        }
    }

    /// Each time its person asks for what `button` names, from now on: at the terminal, each line
    /// entered, until its input ends; on the page, each press of a button so named.
    pub fn presses(self, button: &str) -> Result<Receiver<()>, String> {
        match self {
            Person::Terminal => Ok(enters()),
            Person::Page(page) => {
                let answers = page.ask("", &[button], None, false)?;
                let (pressed, presses) = mpsc::channel();
                thread::spawn(move || {
                    while answers.recv().is_ok() {
                        if pressed.send(()).is_err() {
                            return;
                        }
                    }
                });
                Ok(presses)
            }
        }
    }

    /// How the person asks for a fresh link code.
    pub fn fresh_code(self) -> &'static str {
        match self {
            Person::Terminal => "Press Enter for a fresh code.",
            Person::Page(_) => "Press Fresh code on this page for a fresh one.",
        }
    }
}

/// Each line its person enters, from now on, until its input ends.
fn enters() -> Receiver<()> {
    let (entered, enters) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if line.is_err() || entered.send(()).is_err() {
                return;
            }
        }
    });
    enters
}

/// The line its person enters on `input` after `prompt` on `output`, without its end; `None` once
/// the input ends.
pub fn read(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> io::Result<Option<String>> {
    write!(output, "{prompt}")?;
    output.flush()?;
    let mut line = String::new();
    let read = input.read_line(&mut line)?;
    Ok((read > 0).then(|| line.trim_end_matches(['\r', '\n']).to_owned()))
}

/// Whether its person answers yes to `question` on `input`, asked on `output` again until yes or
/// no; `None` once the input ends.
pub fn ask(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
) -> io::Result<Option<bool>> {
    loop {
        let Some(answer) = read(input, output, &format!("{question} [yes/no]: "))? else {
            writeln!(output)?;
            return Ok(None);
        };
        match answer.trim().to_lowercase().as_str() {
            "yes" | "y" => return Ok(Some(true)),
            "no" | "n" => return Ok(Some(false)),
            _ => writeln!(output, "Please answer yes or no.")?,
        }
    }
}
