//! Who may sign up: anyone a provider signs in, or, while sign-up is closed, only the identities on
//! the operator's lists. Existing accounts sign in and link methods either way.

use crate::sign_in::Identity;

/// The server's sign-up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SignUp {
    /// Anyone a provider signs in creates an account.
    #[default]
    Open,
    /// Only the identities on these lists create accounts.
    Closed(Listed),
}

/// Who may sign up while sign-up is closed; entries compare without case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listed {
    /// GitHub logins: GitHub's public data gives no verified email, so GitHub counts by login only.
    pub github_logins: Vec<String>,
    /// Google emails, which count only when Google has verified them.
    pub google_emails: Vec<String>,
    /// Google `hd` claims, the accounts' Workspace or Cloud organizations; never an email's domain.
    pub google_domains: Vec<String>,
}

/// Whether `list` has `value`, without case.
fn has(list: &[String], value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.to_lowercase();
        list.iter().any(|entry| entry.to_lowercase() == value)
    })
}

impl SignUp {
    /// Whether `identity`, which has no account, may create one.
    pub(crate) fn admits(&self, identity: &Identity) -> bool {
        match self {
            SignUp::Open => true,
            SignUp::Closed(listed) => {
                has(&listed.github_logins, identity.login.as_deref())
                    || has(&listed.google_emails, identity.verified_email.as_deref())
                    || has(&listed.google_domains, identity.hosted_domain.as_deref())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(login: Option<&str>, email: Option<&str>, domain: Option<&str>) -> Identity {
        Identity {
            subject: "1".to_owned(),
            login: login.map(str::to_owned),
            source: String::new(),
            verified_email: email.map(str::to_owned),
            hosted_domain: domain.map(str::to_owned),
        }
    }

    #[test]
    fn open_admits_anyone() {
        assert!(SignUp::Open.admits(&identity(None, None, None)));
    }

    #[test]
    fn closed_admits_each_list_and_nothing_else() {
        let closed = SignUp::Closed(Listed {
            github_logins: vec!["Ada".to_owned()],
            google_emails: vec!["Grace@Example.com".to_owned()],
            google_domains: vec!["Acme.Example".to_owned()],
        });
        assert!(closed.admits(&identity(Some("aDA"), None, None)));
        assert!(closed.admits(&identity(None, Some("grace@EXAMPLE.com"), None)));
        assert!(closed.admits(&identity(None, None, Some("acme.example"))));
        assert!(!closed.admits(&identity(None, None, None)));
        assert!(!closed.admits(&identity(
            Some("bob"),
            Some("bob@acme.example"),
            Some("other")
        )));
        // A login does not match an email or a domain entry, nor the other way round.
        assert!(!closed.admits(&identity(Some("acme.example"), None, None)));
        assert!(!closed.admits(&identity(None, Some("ada"), None)));
        assert!(!SignUp::Closed(Listed::default()).admits(&identity(Some(""), Some(""), Some(""))));
    }
}
