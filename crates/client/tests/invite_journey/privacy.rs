//! Inspect the fixture's traffic, stored rows and server log without displaying their contents.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use skald_crypto::InviteLink;
use zeroize::Zeroizing;

use crate::browser::Page;
use crate::cloud::Cloud;
use crate::keys;
use crate::machine::TestDir;
use crate::spa::Spa;

const LOG_PROBE: &str = "SKALD_TEST_INVITE_LOG_PROBE";
const TEST: &str = "a_new_account_joins_by_link_and_both_accounts_agents_exchange_peer_messages";
const FAILURE: &str = "Invite journey failed at ";

/// The Cloud fixture runs the real server in-process. Run the journey in a child with libtest's
/// capture disabled so that the parent's pipes get every log line, including those from workers.
pub fn with_server_log(journey: fn(&Path)) {
    if let Some(path) = env::var_os(LOG_PROBE) {
        std::panic::set_hook(Box::new(|failure| {
            if let Some(location) = failure.location() {
                println!("{FAILURE}{location}");
            }
        }));
        journey(Path::new(&path));
        return;
    }

    let dir = TestDir::new("invite-log");
    let path = dir.join("observed-link");
    let run = Command::new(env::current_exe().expect("the test binary"))
        .args([TEST, "--exact", "--nocapture"])
        .env(LOG_PROBE, &path)
        .output()
        .expect("run the invite journey");

    // The disposable test credential travels through a private fixture file, never as log output.
    let link = fs::read_to_string(&path).ok();
    if let Some(link) = link {
        fs::remove_file(path).expect("remove the fixture's link");
        let invite = Invite::parse(link);
        invite.assert_clean(&run.stdout, "the server's stdout");
        invite.assert_clean(&run.stderr, "the server's stderr");
    } else {
        assert!(
            !run.status.success(),
            "the log check did not get the test credential"
        );
    }
    let shown = String::from_utf8_lossy(&run.stdout);
    let location = shown
        .lines()
        .find_map(|line| line.strip_prefix(FAILURE))
        .unwrap_or("an unknown location");
    assert!(
        run.status.success() && shown.contains("1 passed"),
        "the invite journey failed at {location}"
    );
}

/// Keep every committed insert and update in the disposable database, even if the original row
/// changes again or is deleted before the privacy check. The row history is read via `Cloud::rows`.
pub fn record_database(cloud: &Cloud) {
    cloud.execute(
        "CREATE SCHEMA invite_journey_audit;
         CREATE TABLE invite_journey_audit.row_writes (recorded_row jsonb NOT NULL);
         CREATE FUNCTION invite_journey_audit.record_write() RETURNS trigger
         LANGUAGE plpgsql SET bytea_output = 'hex' AS $$
         BEGIN
             INSERT INTO invite_journey_audit.row_writes VALUES (to_jsonb(NEW));
             RETURN NULL;
         END;
         $$;",
    );
    // Release each table lock before taking the next, while the server's sweepers are running.
    cloud.execute(
        "DO $$
         DECLARE target record;
         BEGIN
             FOR target IN
                 SELECT table_schema, table_name FROM information_schema.tables
                 WHERE table_type = 'BASE TABLE'
                   AND table_schema NOT IN ('pg_catalog', 'information_schema', 'invite_journey_audit')
                   AND table_name <> '_sqlx_migrations'
             LOOP
                 EXECUTE format('CREATE TRIGGER invite_journey_record_write
                     AFTER INSERT OR UPDATE ON %I.%I FOR EACH ROW
                     EXECUTE FUNCTION invite_journey_audit.record_write()',
                     target.table_schema, target.table_name);
                 COMMIT;
             END LOOP;
         END;
         $$;",
    );
}

pub struct Invite {
    link: Zeroizing<String>,
    parsed: InviteLink,
}

impl Invite {
    pub fn from_terminal(text: &str, spa: &Spa, observed_link: &Path) -> Self {
        let prefix = spa.at("/invite/");
        let shown = text
            .split_whitespace()
            .find(|word| word.starts_with(&prefix))
            .expect("the terminal shows the link at the web address");
        let invite = Self::parse(shown.to_owned());
        fs::write(observed_link, invite.link()).expect("record the link for the log check");
        invite
    }

    fn parse(link: String) -> Self {
        let link = Zeroizing::new(link);
        let parsed = InviteLink::parse(&link).expect("a valid invite link");
        Self { link, parsed }
    }

    pub fn link(&self) -> &str {
        &self.link
    }

    #[track_caller]
    pub fn assert_clean(&self, bytes: &[u8], surface: &str) {
        let (_, fragment) = self.link.split_once('#').expect("the link's fragment");
        assert!(
            !bytes
                .windows(fragment.len())
                .any(|chunk| chunk == fragment.as_bytes())
                && !keys::holds(bytes, self.parsed.secret.to_bytes().as_ref()),
            "the invite secret appeared in {surface}"
        );
    }

    pub fn check_database(&self, cloud: &Cloud) {
        let rows = cloud.rows();
        assert!(
            rows.iter().any(|table| table.contains("<recorded_row>")),
            "the row audit recorded database writes"
        );
        for table in rows {
            self.assert_clean(table.as_bytes(), "database rows and their write history");
        }
    }

    pub fn check_requests(&self, cloud: &Cloud, [admin, newcomer]: [&Page; 2]) {
        let sent = cloud.relay.sent_streams();
        assert!(!sent.is_empty(), "the proxy recorded requests");
        for stream in sent.values() {
            self.assert_clean(stream, "requests at the server proxy");
        }

        let address = self.link.split_once('#').expect("the link's address").0;
        for (page, navigation) in [(admin, "/members"), (newcomer, address)] {
            let requests = page.requests();
            assert!(
                requests
                    .iter()
                    .any(|request| request.method == "GET" && request.url.ends_with(navigation)),
                "the browser recorded the navigation"
            );
            for request in requests {
                self.assert_clean(request.url.as_bytes(), "a browser request URL");
                for (name, value) in request.headers {
                    self.assert_clean(name.as_bytes(), "a browser request header name");
                    self.assert_clean(value.as_bytes(), "a browser request header value");
                }
                if let Some(body) = request.body {
                    let bytes: Vec<u8> = body.chars().map(|ch| ch as u8).collect();
                    self.assert_clean(&bytes, "a browser request body");
                }
            }
        }
    }
}
