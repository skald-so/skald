//! The web app's Account page, as a person uses it in the browser.
//!
//! The web app, as these tests use it, beside `common/spa.rs` and `web_sessions/access.rs`:
//! - the account menu, the footer's button "Account", has the item "Account", which opens the
//!   Account page: the username with the GitHub login next to it; each sign-in method of the
//!   account, naming its provider, "GitHub" or "Google", when it was linked and, for GitHub, its
//!   login; while the account has two or more methods, a button whose name starts with "Remove" for
//!   each, in the method's table row or list item or naming its provider; and a button "Link
//!   GitHub" or "Link Google" for each provider the server has settings for, which links a method
//!   of that provider and returns to the Account page;
//! - Remove asks first: a dialog, role `dialog` or `alertdialog`, whose button whose name starts
//!   with "Remove" removes the method. For the method this web session signed in with, the dialog
//!   says that it signs "this browser" out;
//! - with a sign-in older than `--link-sign-in-age`, Remove removes nothing, and the page asks to
//!   sign in again with a link or button whose text says "sign in".

use serde_json::{Value, json};

use super::access::press;
use super::browser::Page;
use super::spa::wait_until;
use crate::Provider;

/// The dialogs a confirmation opens.
const DIALOG: &str = "[role=dialog], [role=alertdialog]";

/// The address of the page open in `page`.
pub fn location(page: &Page) -> String {
    let at = page.evaluate("location.href");
    at.as_str().unwrap_or_default().to_owned()
}

/// Opens the Account page from the account menu of the cabinet in `page`, and waits for its link
/// buttons.
pub fn open_account(page: &Page) {
    press(page, None, "button", "^account$");
    press(page, None, "[role=menuitem]", "^account$");
    wait_until(page, "the Account page", |page| {
        page.count(crate::LINK_GITHUB) + page.count(crate::LINK_GOOGLE) > 0
    });
}

/// A script that gives, for each "Remove" button outside a dialog in the page, the text that
/// names its method: its table row's or list item's, and its own name.
const REMOVES: &str = r#"[...document.querySelectorAll("button")]
    .filter((button) => !button.closest("[role=dialog], [role=alertdialog]"))
    .map((button) => [button, (button.getAttribute("aria-label") ?? button.textContent).trim()])
    .filter(([, name]) => /^remove\b/i.test(name))
    .map(([button, name]) => {
        const scope = button.closest("tr, [role=row], li, [role=listitem]");
        return [button, `${scope ? scope.innerText : ""} ${name}`];
    })"#;

/// How many "Remove" buttons the Account page in `page` shows.
pub fn remove_count(page: &Page) -> usize {
    let count = page.evaluate(&format!("({REMOVES}).length"));
    count.as_u64().expect("a count") as usize
}

/// The word that names `provider` on the page.
fn word(provider: Provider) -> &'static str {
    match provider {
        Provider::GitHub => "github",
        Provider::Google => "google",
    }
}

/// Clicks "Remove" of the method of `provider` on the Account page in `page`; gives the text of the
/// dialog it opens, or `None` if the page asks to sign in again instead.
#[track_caller]
pub fn ask_to_remove(page: &Page, provider: Provider) -> Option<String> {
    let other = match provider {
        Provider::GitHub => Provider::Google,
        Provider::Google => Provider::GitHub,
    };
    let asking = sign_in_prompts(page);
    let clicked = page.evaluate(&format!(
        r#"(() => {{
            const found = ({REMOVES}).filter(([, text]) =>
                text.toLowerCase().includes({mine}) && !text.toLowerCase().includes({other}));
            if (found.length !== 1) return found.length;
            found[0][0].click();
            return true;
        }})()"#,
        mine = json!(word(provider)),
        other = json!(word(other)),
    ));
    assert_eq!(
        clicked,
        json!(true),
        "one Remove of {provider:?}; the page shows: {:?}",
        page.text()
    );
    let shown = page.evaluate(&format!(
        r#"(async () => {{
            const deadline = Date.now() + 10000;
            while (Date.now() < deadline) {{
                const dialog = document.querySelector({DIALOG:?});
                if (dialog) return dialog.innerText;
                if ({SIGN_IN_PROMPTS} > {asking}) return false;
                await new Promise((resolve) => setTimeout(resolve, 50));
            }}
            return null;
        }})()"#
    ));
    match shown {
        Value::String(text) => Some(text),
        Value::Bool(false) => None,
        _ => panic!("no dialog; the page shows: {:?}", page.text()),
    }
}

/// A script that counts the links and buttons whose text says "sign in".
const SIGN_IN_PROMPTS: &str = r#"[...document.querySelectorAll("a, button")]
    .filter((element) => /sign in/i.test(element.textContent)).length"#;

/// How many links and buttons whose text says "sign in" `page` shows.
pub fn sign_in_prompts(page: &Page) -> usize {
    let count = page.evaluate(SIGN_IN_PROMPTS);
    count.as_u64().expect("a count") as usize
}

/// Confirms the removal in the dialog in `page` by its button whose name starts with "Remove".
pub fn confirm_removal(page: &Page) {
    press(page, Some(DIALOG), "button", r"^remove\b");
}
