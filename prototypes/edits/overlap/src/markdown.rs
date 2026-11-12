//! The Markdown of the commands' reports: tables and lists.

use std::fmt::{Display, Write};

/// Appends to `out` the Markdown table `title` with `header`, after an empty line, and the rows
/// `rows` passes to the function it takes, each a list of cells.
pub fn table(
    out: &mut String,
    title: &str,
    header: &[String],
    rows: impl FnOnce(&mut dyn FnMut(Vec<String>)),
) {
    let line = |cells: &[String]| format!("| {} |\n", cells.join(" | "));
    write!(out, "\n{title}\n\n{}", line(header)).expect("a String takes any write");
    *out += &line(&vec!["---".to_owned(); header.len()]).replace(' ', "");
    rows(&mut |cells| *out += &line(&cells));
}

/// `items` joined with `, `, or `none` when there are none.
pub fn list(items: impl IntoIterator<Item = impl Display>) -> String {
    let items: Vec<String> = items.into_iter().map(|item| item.to_string()).collect();
    if items.is_empty() {
        "none".to_owned()
    } else {
        items.join(", ")
    }
}
