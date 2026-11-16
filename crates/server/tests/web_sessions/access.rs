//! The web app's Access section and account menu, as a person uses them in the browser. The
//! client's tests include this file too, beside `common/browser.rs` and `common/spa.rs`.
//!
//! The web app, as these tests use it, beside `common/spa.rs`:
//! - the side menu's link "Access" opens the section: a table with a row per session of the
//!   account; the row of the page's own session says "this session" and has no "Revoke" button;
//!   each other row has one, which ends its session at once, without a confirmation. A web
//!   session's row says when it expires; a CLI session's row says "when revoked" and names its
//!   device;
//! - the section's button "Sign out everywhere else" opens a dialog, role `dialog` or
//!   `alertdialog`, that says the machines are signed out too, and ends nothing until its button
//!   whose name says "sign out" is clicked;
//! - the account menu, the footer's button "Account", has the item "Sign out", which ends the
//!   page's web session and shows the sign-in page.

use serde_json::{Value, json};

use super::browser::Page;
use super::spa::wait_until;

/// What the row of the page's own session says.
pub const THIS_SESSION: &str = "this session";
/// What a CLI session's row says it expires.
pub const WHEN_REVOKED: &str = "when revoked";

/// Clicks, as a person, the first element that `selector` finds within the first element `scope`
/// finds, the page if `None`, whose accessible name, its `aria-label` or else its text, matches the
/// case-insensitive regular expression `name`; waits up to 10 s for it to show. Gives the text of
/// the scope it clicked in.
#[track_caller]
pub fn press(page: &Page, scope: Option<&str>, selector: &str, name: &str) -> String {
    let script = format!(
        r#"(async () => {{
            const named = (element) =>
                new RegExp({name}, "i").test(
                    (element.getAttribute("aria-label") ?? element.textContent).trim());
            const deadline = Date.now() + 10000;
            for (;;) {{
                const within = {scope} === null ? document.body : document.querySelector({scope});
                const found = within && [...within.querySelectorAll({selector})].find(named);
                if (found) {{
                    const text = within.innerText;
                    found.click();
                    return text;
                }}
                if (Date.now() > deadline) return null;
                await new Promise((resolve) => setTimeout(resolve, 50));
            }}
        }})()"#,
        name = json!(name),
        scope = json!(scope),
        selector = json!(selector),
    );
    match page.evaluate(&script) {
        Value::String(text) => text,
        _ => panic!(
            "no {selector} named /{name}/ in {scope:?}; the page shows: {:?}",
            page.text()
        ),
    }
}

/// The text of each row of the tables in `page`, header rows left out.
pub fn rows(page: &Page) -> Vec<String> {
    let found = page.evaluate(
        r#"[...document.querySelectorAll("tr, [role=row]")]
            .filter((row) => !row.querySelector("th, [role=columnheader]"))
            .map((row) => row.innerText)"#,
    );
    serde_json::from_value(found).expect("the rows' text")
}

/// The rows of `page` whose text holds `part`, in any case.
pub fn rows_holding(page: &Page, part: &str) -> Vec<String> {
    let mut found = rows(page);
    found.retain(|row| row.to_lowercase().contains(&part.to_lowercase()));
    found
}

/// Opens the Access section from the side menu of the cabinet in `page`, and waits for the row of
/// the page's own session.
pub fn open_access(page: &Page) {
    press(page, None, "a, [role=link]", "^access$");
    wait_until(page, "the Access section", |page| {
        rows_holding(page, THIS_SESSION).len() == 1
    });
}

/// Clicks "Revoke" in the one row of the Access section in `page` whose text holds `part`.
#[track_caller]
pub fn revoke(page: &Page, part: &str) {
    let found = rows_holding(page, part);
    assert_eq!(found.len(), 1, "one row holding {part:?}: {:?}", rows(page));
    let clicked = page.evaluate(&format!(
        r#"(() => {{
            const row = [...document.querySelectorAll("tr, [role=row]")].find((row) =>
                row.innerText.toLowerCase().includes({part}));
            const button = row && [...row.querySelectorAll("button")].find((button) =>
                /^revoke\b/i.test((button.getAttribute("aria-label") ?? button.textContent).trim()));
            if (!button) return false;
            button.click();
            return true;
        }})()"#,
        part = json!(part.to_lowercase()),
    ));
    assert_eq!(clicked, json!(true), "no Revoke in {found:?}");
}

/// The dialogs a confirmation opens.
const DIALOG: &str = "[role=dialog], [role=alertdialog]";

/// Clicks "Sign out everywhere else" in the Access section in `page`; gives the text of the dialog
/// it opens, which asks first.
pub fn ask_to_sign_out_everywhere_else(page: &Page) -> String {
    press(page, None, "button", "^sign out everywhere else$");
    let shown = page.evaluate(&format!(
        r#"(async () => {{
            const deadline = Date.now() + 10000;
            while (!document.querySelector({DIALOG:?}) && Date.now() < deadline)
                await new Promise((resolve) => setTimeout(resolve, 50));
            return document.querySelector({DIALOG:?})?.innerText ?? null;
        }})()"#
    ));
    match shown {
        Value::String(text) => text,
        _ => panic!("no dialog; the page shows: {:?}", page.text()),
    }
}

/// Confirms the dialog in `page` by its button whose name says "sign out".
pub fn confirm_sign_out(page: &Page) {
    press(page, Some(DIALOG), "button", "sign out");
}

/// Clicks the one "Revoke" button in `page`: that of the one session other than the page's own.
#[track_caller]
pub fn revoke_the_other(page: &Page) {
    let buttons = page.count(REVOKE);
    assert_eq!(buttons, 1, "one Revoke: {:?}", rows(page));
    press(page, None, "button", r"^revoke\b");
}

/// The "Revoke" buttons of the Access section.
pub const REVOKE: &str = r#"role=button[name=/^revoke\b/i]"#;

/// Signs the page's web session out from the account menu in `page`.
pub fn sign_out(page: &Page) {
    press(page, None, "button", "^account$");
    press(page, None, "[role=menuitem]", "^sign out$");
}
