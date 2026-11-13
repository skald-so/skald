//! GitHub's OAuth token endpoint and user endpoint, as an OAuth App reaches them.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use url::Url;

use crate::http_client::agent;

/// The REST API version the server reads the user with.
const API_VERSION: &str = "2022-11-28";

/// A base address of GitHub, such as `https://github.com`, without a trailing slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubUrl(String);

impl GitHubUrl {
    /// The address of `path`, which starts with `/`, under this base.
    pub(crate) fn at(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }
}

/// An http or https URL without a user, query or fragment.
impl FromStr for GitHubUrl {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let url = http_url(text)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("a URL with a user name or password".to_owned());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("a URL with a query or fragment".to_owned());
        }
        Ok(GitHubUrl(url.as_str().trim_end_matches('/').to_owned()))
    }
}

/// `text` as an http or https URL.
pub(crate) fn http_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|err| format!("not a URL: {err}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("not an http or https URL".to_owned());
    }
    Ok(url)
}

/// An OAuth client's secret: a GitHub OAuth App's or an OIDC client's; never printed.
#[derive(Clone)]
pub struct ClientSecret(String);

impl ClientSecret {
    /// The secret `text`; `None` if it is empty.
    pub fn new(text: String) -> Option<Self> {
        (!text.is_empty()).then_some(ClientSecret(text))
    }
}

impl fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientSecret(…)")
    }
}

/// The GitHub OAuth App the server signs people in with, and GitHub's addresses.
#[derive(Debug, Clone)]
pub struct GitHubApp {
    pub client_id: String,
    pub client_secret: ClientSecret,
    /// GitHub's address: the authorize and token endpoints.
    pub url: GitHubUrl,
    /// GitHub's API address: the user endpoint.
    pub api_url: GitHubUrl,
}

/// A GitHub user, as the user endpoint gives it.
#[derive(Debug, Deserialize)]
pub(crate) struct User {
    pub id: u64,
    pub login: String,
}

/// Why GitHub gave no user.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// GitHub refused the code: wrong, expired or used.
    Code,
    /// GitHub failed, or refused the app.
    GitHub(String),
}

impl GitHubApp {
    /// The user who authorized `code`, which GitHub gave the callback `redirect_uri`. Blocks; the
    /// GitHub token it gets is dropped once it has read the user.
    pub(crate) fn user(&self, code: &str, redirect_uri: &str) -> Result<User, Refusal> {
        let token = self.token(code, redirect_uri)?;
        self.user_of(&token)
    }

    /// Exchanges `code` for a token at the token endpoint.
    fn token(&self, code: &str, redirect_uri: &str) -> Result<String, Refusal> {
        #[derive(Deserialize)]
        struct Answer {
            access_token: Option<String>,
            error: Option<String>,
        }
        let response = agent()
            .post(self.url.at("/login/oauth/access_token"))
            .header("accept", "application/json")
            .send_form([
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.0.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ]);
        let answer: Answer = read("token endpoint", response)?;
        match (answer.access_token, answer.error) {
            (Some(token), None) => Ok(token),
            (_, Some(error)) if error == "bad_verification_code" => Err(Refusal::Code),
            (_, error) => Err(Refusal::GitHub(format!(
                "GitHub's token endpoint gave no token: {}",
                printable(error.as_deref().unwrap_or("no error"))
            ))),
        }
    }

    /// The user of `token`.
    fn user_of(&self, token: &str) -> Result<User, Refusal> {
        let response = agent()
            .get(self.api_url.at("/user"))
            .header("accept", "application/vnd.github+json")
            .header("authorization", format!("Bearer {token}"))
            .header("x-github-api-version", API_VERSION)
            .call();
        read("user endpoint", response)
    }
}

/// The JSON body of a `200` answer of GitHub's `endpoint`.
fn read<T: for<'de> Deserialize<'de>>(
    endpoint: &str,
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<T, Refusal> {
    let failed = |reason: String| Refusal::GitHub(format!("GitHub's {endpoint} {reason}"));
    let mut response = response.map_err(|err| failed(format!("is unreachable: {err}")))?;
    let status = response.status();
    if status != 200 {
        return Err(failed(format!("answered {status}")));
    }
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| failed(format!("gave an unreadable answer: {err}")))?;
    serde_json::from_str(&body).map_err(|err| failed(format!("gave an unexpected answer: {err}")))
}

/// `text` from GitHub, such as an error code, cut to printable ASCII for the server's output.
fn printable(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_graphic())
        .take(64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_url_drops_the_trailing_slash() {
        let url: GitHubUrl = "https://ghe.example/api/v3/".parse().expect("a URL");
        assert_eq!(url.at("/user"), "https://ghe.example/api/v3/user");
    }

    #[test]
    fn github_url_refuses_what_is_not_a_base_address() {
        for text in [
            "github.com",
            "ftp://github.com",
            "https://user@github.com",
            "https://:pass@github.com",
            "https://github.com/?q=1",
            "https://github.com/#top",
        ] {
            assert!(text.parse::<GitHubUrl>().is_err(), "{text}");
        }
    }

    #[test]
    fn printable_keeps_printable_ascii_only() {
        assert_eq!(printable("bad\n code\u{1b}[31m"), "badcode[31m");
        assert_eq!(printable(&"x".repeat(100)).len(), 64);
    }

    #[test]
    fn client_secret_is_never_printed() {
        let secret = ClientSecret::new("s3cret".to_owned()).expect("a secret");
        assert_eq!(format!("{secret:?}"), "ClientSecret(…)");
        assert!(ClientSecret::new(String::new()).is_none());
    }
}
