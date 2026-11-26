//! Select the fixture space on the Members page.

use crate::access::press;
use crate::browser::Page;

pub fn select_acme(page: &Page<'_>, phone: bool) {
    if phone {
        press(page, None, "button", "^menu$");
    }
    press(page, None, "button", "^space\\b");
    press(page, None, "[role=option]", "^acme\\b");
    if phone {
        press(page, None, "button", "^menu$");
    }
}
