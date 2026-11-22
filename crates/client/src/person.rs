//! Where a command takes its person's answers and shows what only the person may see, such as a
//! link code or a recovery key: at the person's terminal, or on the local page that `--browser`
//! opens ([`crate::page`]), never in the command's output.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

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
            Person::Terminal => ask(&mut input(), &mut io::stdout(), question)
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
            Person::Terminal => read(&mut input(), &mut io::stdout(), &format!("{label}: "))
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
    pub fn presses(self, button: &str) -> Result<Presses, String> {
        match self {
            Person::Terminal => {
                lines();
                Ok(Presses::Terminal)
            }
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
                Ok(Presses::Page(presses))
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

/// What its person asks for, from [`Person::presses`]; `Never` for a person who asks for nothing.
pub enum Presses {
    Terminal,
    Page(Receiver<()>),
    Never,
}

/// What its person did since the last look.
#[derive(Debug, PartialEq)]
pub enum Pressed {
    Once,
    Not,
    /// Its input ended: it asks for nothing more.
    Ended,
}

impl Presses {
    /// Whether its person asked once more since the last look.
    pub fn look(&self) -> Pressed {
        let taken = match self {
            Presses::Terminal => lines()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_recv()
                .map(drop),
            Presses::Page(presses) => presses.try_recv(),
            Presses::Never => Err(TryRecvError::Empty),
        };
        match taken {
            Ok(()) => Pressed::Once,
            Err(TryRecvError::Empty) => Pressed::Not,
            Err(TryRecvError::Disconnected) => Pressed::Ended,
        }
    }

    /// Forgets what its person asked for so far.
    pub fn drain(&self) {
        while self.look() == Pressed::Once {}
    }
}

/// The lines its person enters at the terminal, read by one thread once something waits for them
/// while it goes on, so that each later read takes its line from there.
static LINES: OnceLock<Mutex<Receiver<String>>> = OnceLock::new();

/// Whether the thread that reads its person's lines holds off: then another program, such as
/// `ssh-keygen` asking for a passphrase, reads the terminal.
static HELD_OFF: AtomicBool = AtomicBool::new(false);

/// How long the thread waits for input before it looks whether it holds off.
const LOOK: Duration = Duration::from_millis(50);

/// Holds off the thread that reads its person's lines until it is dropped; a line it read before
/// stays its own.
pub struct HoldOff(());

/// [`HoldOff`].
pub fn hold_off() -> HoldOff {
    HELD_OFF.store(true, Ordering::SeqCst);
    HoldOff(())
}

impl Drop for HoldOff {
    fn drop(&mut self) {
        HELD_OFF.store(false, Ordering::SeqCst);
    }
}

/// The lines its person enters from now on, until its input ends. The thread reads only input
/// that waits, and none while it holds off ([`hold_off`]).
fn lines() -> &'static Mutex<Receiver<String>> {
    LINES.get_or_init(|| {
        let (entered, lines) = mpsc::channel();
        thread::spawn(move || {
            let mut stdin = io::stdin().lock();
            loop {
                if HELD_OFF.load(Ordering::SeqCst) || !input_waits(LOOK) {
                    thread::sleep(LOOK);
                    continue;
                }
                let mut line = String::new();
                match stdin.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let line = line.strip_suffix('\n').unwrap_or(&line);
                let line = line.strip_suffix('\r').unwrap_or(line);
                if entered.send(line.to_owned()).is_err() {
                    return;
                }
            }
        });
        Mutex::new(lines)
    })
}

/// Whether the standard input has input waiting, or its end or an error, within `timeout`.
#[cfg(unix)]
fn input_waits(timeout: Duration) -> bool {
    let mut stdin = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: one valid pollfd, for the time of the call.
    let polled = unsafe { libc::poll(&raw mut stdin, 1, timeout) };
    polled != 0
}

/// Whether the standard input has a line waiting, or its end or an error, within `timeout`. On a
/// console, a line waits once its input records hold a press of Enter: any record, such as a key's
/// release or a change of focus, signals the console, and reading a line then would block and take
/// the next line typed, even one another program asks for.
#[cfg(windows)]
fn input_waits(timeout: Duration) -> bool {
    use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetNumberOfConsoleInputEvents, GetStdHandle, INPUT_RECORD, KEY_EVENT,
        PeekConsoleInputW, STD_INPUT_HANDLE,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    let timeout = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: the process's standard input handle, which it does not close; the records peeked
    // fit the buffer, and a key event's record is read as one.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        if WaitForSingleObject(input, timeout) == WAIT_TIMEOUT {
            return false;
        }
        let mut mode = 0;
        let mut count = 0;
        if GetConsoleMode(input, &raw mut mode) == 0
            || GetNumberOfConsoleInputEvents(input, &raw mut count) == 0
        {
            return true;
        }
        let mut records = vec![INPUT_RECORD::default(); count as usize];
        let mut peeked = 0;
        if PeekConsoleInputW(input, records.as_mut_ptr(), count, &raw mut peeked) == 0 {
            return true;
        }
        records.iter().take(peeked as usize).any(|record| {
            u32::from(record.EventType) == KEY_EVENT && {
                let key = record.Event.KeyEvent;
                key.bKeyDown != 0 && key.uChar.UnicodeChar == u16::from(b'\r')
            }
        })
    }
}

/// Shows its person a prompt with `show`, then gives the line entered after it, without its end;
/// `None` once the input ends. Lines entered before are forgotten first, so that the one entered as
/// the prompt shows is the answer.
pub fn answer(show: impl FnOnce() -> Result<(), String>) -> Result<Option<String>, String> {
    answer_from(LINES.get(), show)
}

/// [`answer`], from `lines` once a thread reads them; otherwise from the standard input.
fn answer_from(
    lines: Option<&'static Mutex<Receiver<String>>>,
    show: impl FnOnce() -> Result<(), String>,
) -> Result<Option<String>, String> {
    if let Some(lines) = lines {
        let lines = lines.lock().unwrap_or_else(PoisonError::into_inner);
        while lines.try_recv().is_ok() {}
    }
    show()?;
    let mut input = match lines {
        Some(lines) => Input::Lines(lines, Vec::new()),
        None => Input::Stdin(io::stdin().lock()),
    };
    let mut line = String::new();
    let read =
        (input.read_line(&mut line)).map_err(|err| format!("cannot read the terminal: {err}"))?;
    Ok((read > 0).then(|| line.trim_end_matches(['\r', '\n']).to_owned()))
}

/// What its person enters at the terminal: from the thread that reads its lines, once one does;
/// otherwise the standard input.
pub fn input() -> Input {
    match LINES.get() {
        Some(lines) => Input::Lines(lines, Vec::new()),
        None => Input::Stdin(io::stdin().lock()),
    }
}

/// The terminal's input, as [`input`] gives it.
pub enum Input {
    Stdin(io::StdinLock<'static>),
    /// The lines the thread read, and the rest of the line taken.
    Lines(&'static Mutex<Receiver<String>>, Vec<u8>),
}

impl io::Read for Input {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let taken = available.len().min(buf.len());
        buf[..taken].copy_from_slice(&available[..taken]);
        self.consume(taken);
        Ok(taken)
    }
}

impl BufRead for Input {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match self {
            Input::Stdin(stdin) => stdin.fill_buf(),
            Input::Lines(lines, rest) => {
                if rest.is_empty() {
                    let line = lines.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    if let Ok(line) = line {
                        *rest = format!("{line}\n").into_bytes();
                    }
                }
                Ok(rest)
            }
        }
    }

    fn consume(&mut self, amount: usize) {
        match self {
            Input::Stdin(stdin) => stdin.consume(amount),
            Input::Lines(_, rest) => {
                rest.drain(..amount.min(rest.len()));
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const QUESTION: &str = "Link it to this account?";

    fn asked(input: &str) -> (Option<bool>, String) {
        let mut output = Vec::new();
        let answer = ask(&mut input.as_bytes(), &mut output, QUESTION).expect("asked");
        (answer, String::from_utf8(output).expect("UTF-8"))
    }

    /// The answer is the line entered as the prompt shows; one entered before it is forgotten.
    #[test]
    fn answer_is_the_line_entered_as_the_prompt_shows() {
        let (entered, lines) = mpsc::channel();
        let lines = Box::leak(Box::new(Mutex::new(lines)));
        entered.send("before".to_owned()).unwrap();
        let answer = answer_from(Some(lines), move || {
            entered.send("as it shows".to_owned()).unwrap();
            // The input ends: an answer forgotten gives none rather than a wait.
            drop(entered);
            Ok(())
        });
        assert_eq!(answer, Ok(Some("as it shows".to_owned())));
    }

    #[test]
    fn question_takes_yes_or_no_and_asks_again_otherwise() {
        for (input, answer) in [
            ("yes\n", Some(true)),
            ("Y\n", Some(true)),
            (" no \n", Some(false)),
            ("n", Some(false)),
            ("maybe\nyes\n", Some(true)),
            ("", None),
            ("maybe\n", None),
        ] {
            let (got, shown) = asked(input);
            assert_eq!(got, answer, "{input:?}");
            assert!(shown.contains(QUESTION), "{shown}");
        }
        assert!(asked("maybe\nno\n").1.contains("Please answer yes or no."));
    }
}
