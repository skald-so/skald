//! The public HTTP requests of a synthetic device and its sign-in.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use skald_github_stand_in::{GitHub, User};
use skald_proto::{PROTOCOL_VERSION, VERSION_HEADER};
use ureq::{Agent, Body, http::Response};
use url::Url;

use crate::Result;

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn unb64(bytes: &str) -> Result<Vec<u8>> {
    Ok(URL_SAFE_NO_PAD.decode(bytes)?)
}

pub fn json(mut response: Response<Body>) -> Result<Value> {
    Ok(serde_json::from_slice(&response.body_mut().read_to_vec()?)?)
}

pub fn text<'a>(json: &'a Value, field: &str) -> Result<&'a str> {
    json[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}").into())
}

pub struct Http {
    pub origin: String,
    agent: Agent,
}

impl Http {
    pub fn new(origin: String) -> Self {
        let config = Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build();
        Self {
            origin,
            agent: config.into(),
        }
    }

    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Response<Body>> {
        let mut request = ureq::http::Request::builder()
            .method(method)
            .uri(url)
            .header(VERSION_HEADER, PROTOCOL_VERSION.to_string());
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        Ok(self.agent.run(request.body(body)?)?)
    }

    pub fn api(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Response<Body>> {
        let authorization = format!("Bearer {token}");
        let headers = [
            ("authorization", authorization.as_str()),
            ("content-type", "application/json"),
        ];
        let body = body
            .map(serde_json::to_vec)
            .transpose()?
            .unwrap_or_default();
        expect_ok(
            self.request(method, &format!("{}{path}", self.origin), &headers, &body)?,
            path,
        )
    }

    fn form(&self, path: &str, cookie: &str, fields: &[(&str, &str)]) -> Result<Response<Body>> {
        let body: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let headers = [
            ("origin", self.origin.as_str()),
            ("cookie", cookie),
            ("content-type", "application/x-www-form-urlencoded"),
        ];
        expect_ok(
            self.request(
                "POST",
                &format!("{}{path}", self.origin),
                &headers,
                body.as_bytes(),
            )?,
            path,
        )
    }

    pub fn sign_in(&self, github: &GitHub, user: User) -> Result<String> {
        let login = github.sign_in_url(user.id);
        github.put_user(user);
        let started = self.request("GET", &format!("{}/auth/github", self.origin), &[], &[])?;
        let authorize = redirect(&started, github.url(), "/login/oauth/authorize")?;
        let started_cookie = cookie(&started, Some("skald_sign_in"))?;
        let signed_in = self.request("GET", &login, &[], &[])?;
        let provider_cookie = cookie(&signed_in, None)?;
        let back = self.request("GET", &authorize, &[("cookie", provider_cookie)], &[])?;
        let callback = redirect(&back, &self.origin, "/auth/github/callback")?;
        let signed_in = expect_ok(
            self.request("GET", &callback, &[("cookie", started_cookie)], &[])?,
            "GitHub callback",
        )?;
        let session = cookie(&signed_in, Some("skald_session"))?;

        let grant = json(self.form("/auth/device", "", &[("client_id", "skald")])?)?;
        let interval = grant["interval"]
            .as_u64()
            .ok_or("missing polling interval")?;
        if interval == 0 || interval > 5 {
            return Err("unexpected fixture polling interval".into());
        }
        self.form(
            "/device",
            session,
            &[
                ("user_code", text(&grant, "user_code")?),
                ("decision", "approve"),
            ],
        )?;
        std::thread::sleep(Duration::from_secs(interval));
        let token = json(self.form(
            "/auth/token",
            "",
            &[
                ("client_id", "skald"),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", text(&grant, "device_code")?),
            ],
        )?)?;
        Ok(text(&token, "access_token")?.to_owned())
    }
}

fn expect_ok(response: Response<Body>, path: &str) -> Result<Response<Body>> {
    if !response.status().is_success() {
        return Err(format!("{path} returned {}", response.status().as_u16()).into());
    }
    Ok(response)
}

fn redirect(response: &Response<Body>, origin: &str, path: &str) -> Result<String> {
    if !response.status().is_redirection() {
        return Err(format!("expected redirect to {path}, got {}", response.status()).into());
    }
    let location = response
        .headers()
        .get("location")
        .ok_or("missing redirect location")?;
    let url = Url::parse(location.to_str()?)?;
    if url.origin().ascii_serialization() != origin
        || url.path() != path
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(format!("redirect did not stay in the fixture at {path}").into());
    }
    Ok(url.into())
}

fn cookie<'a>(response: &'a Response<Body>, name: Option<&str>) -> Result<&'a str> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|header| header.to_str().ok()?.split(';').next())
        .find(|cookie| {
            name.is_none_or(|name| cookie.split_once('=').is_some_and(|(key, _)| key == name))
        })
        .ok_or_else(|| "missing fixture sign-in cookie".into())
}
