//! The browser and OS a `User-Agent` names, as the account's sessions list a web session's sign-in.
//! Only their names, no versions: enough to tell a person's browsers apart.

/// The browser and the OS that `agent`, a `User-Agent` header, names; `None` for one it does not.
pub(crate) fn browser_and_os(agent: &str) -> (Option<&'static str>, Option<&'static str>) {
    (browser(agent), os(agent))
}

/// The browser: the first of these whose token `agent` holds. Browsers built on Chrome or Safari
/// also name those, so their own tokens come first.
fn browser(agent: &str) -> Option<&'static str> {
    const BROWSERS: [(&str, &str); 10] = [
        ("Edg/", "Edge"),
        ("EdgiOS/", "Edge"),
        ("EdgA/", "Edge"),
        ("OPR/", "Opera"),
        ("SamsungBrowser/", "Samsung Internet"),
        ("YaBrowser/", "Yandex Browser"),
        ("Firefox/", "Firefox"),
        ("FxiOS/", "Firefox"),
        ("Chrome/", "Chrome"),
        ("CriOS/", "Chrome"),
    ];
    BROWSERS
        .iter()
        .find(|(token, _)| agent.contains(token))
        .map(|(_, name)| *name)
        .or_else(|| (agent.contains("Safari/") && agent.contains("Version/")).then_some("Safari"))
}

/// The OS: the first of these whose token `agent` holds. Android and ChromeOS also name Linux, and
/// iOS names Mac OS X.
fn os(agent: &str) -> Option<&'static str> {
    const SYSTEMS: [(&str, &str); 8] = [
        ("iPhone", "iOS"),
        ("iPad", "iPadOS"),
        ("Android", "Android"),
        ("CrOS", "ChromeOS"),
        ("Windows", "Windows"),
        ("Macintosh", "macOS"),
        ("Mac OS X", "macOS"),
        ("Linux", "Linux"),
    ];
    SYSTEMS
        .iter()
        .find(|(token, _)| agent.contains(token))
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_browser_and_os_of_common_agents() {
        for (agent, expected) in [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like \
                 Gecko) Chrome/131.0.0.0 Safari/537.36",
                ("Chrome", "macOS"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0",
                ("Firefox", "Windows"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
                ("Edge", "Windows"),
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like \
                 Gecko) Version/18.1 Safari/605.1.15",
                ("Safari", "macOS"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
                ("Safari", "iOS"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) CriOS/131.0.6778.73 Mobile/15E148 Safari/604.1",
                ("Chrome", "iOS"),
            ),
            (
                "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/131.0.0.0 Mobile Safari/537.36",
                ("Chrome", "Android"),
            ),
            (
                "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/131.0.0.0 Safari/537.36",
                ("Chrome", "ChromeOS"),
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/131.0.0.0 Safari/537.36 OPR/114.0.0.0",
                ("Opera", "Linux"),
            ),
        ] {
            let (browser, os) = expected;
            assert_eq!(browser_and_os(agent), (Some(browser), Some(os)), "{agent}");
        }
    }

    #[test]
    fn names_nothing_of_an_unknown_agent() {
        for agent in ["", "curl/8.7.1", "skald/0.1.0"] {
            assert_eq!(browser_and_os(agent), (None, None), "{agent}");
        }
    }
}
