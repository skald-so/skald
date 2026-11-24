//! Small runs against an owned loopback server and a fresh disposable test database.

#[path = "../../server/tests/common/connections.rs"]
mod connections;
mod relay;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use connections::Database;
use relay::{Fault, Relay};
use skald_github_stand_in::{App, GitHub};
use skald_load_delivery::{Delivery, FIXTURE_SEND_LIMIT, FIXTURE_SERVER_NAME, Report, Settings};
use skald_server::{ClientSecret, GitHubApp, Settings as ServerSettings, SignInSettings};
use sqlx::{Connection, PgConnection};
use tokio::runtime::Runtime;
use url::Url;

struct Fixture {
    runtime: Option<Runtime>,
    db: Database,
    address: SocketAddrV4,
    github: GitHub,
    relay: Option<Relay>,
}

impl Fixture {
    fn start() -> Self {
        Self::start_with(None)
    }

    fn start_with(fault: Option<Fault>) -> Self {
        let postgres =
            std::env::var("SKALD_TEST_DATABASE_URL").expect("run this test with script/test");
        let endpoint = Url::parse(&postgres).unwrap();
        assert_eq!(endpoint.host_str(), Some("127.0.0.1"));
        assert!(endpoint.query().is_none());
        let name = format!("skald_load_{}", skald_proto::MessageId::generate()).replace('-', "_");
        let db = Database::create(postgres, name, |url| {
            Box::pin(async move { PgConnection::connect(&url).await })
        });
        assert!(db.name().starts_with("skald_load_"));
        let runtime = Runtime::new().unwrap();
        let (listener, pool) = runtime.block_on(async {
            let pool = skald_server::connect(&db.url()).await.unwrap();
            skald_server::MIGRATOR.run(&pool).await.unwrap();
            let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            (listener, pool)
        });
        let SocketAddr::V4(address) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let public = format!("http://{address}");
        let client_id = "fixture".to_owned();
        let secret = "test-only".to_owned();
        let github = GitHub::start(App {
            client_id: client_id.clone(),
            client_secret: secret.clone(),
            callback: Some(format!("{public}/auth/github/callback")),
        });
        let settings = ServerSettings {
            server_name: Some(FIXTURE_SERVER_NAME.parse().unwrap()),
            sign_in: Some(SignInSettings {
                public_url: public.parse().unwrap(),
                web_url: None,
                github: Some(GitHubApp {
                    client_id,
                    client_secret: ClientSecret::new(secret).unwrap(),
                    url: github.url().parse().unwrap(),
                    api_url: github.url().parse().unwrap(),
                }),
                google: None,
                corporate: None,
                state_lifetime: Duration::from_secs(30),
                link_age: Duration::from_secs(30),
                device_code_lifetime: Duration::from_secs(30),
                device_poll_interval: Duration::from_secs(1),
            }),
            send_rate_limit: Some(FIXTURE_SEND_LIMIT),
            send_rate_window: Some(Duration::from_secs(1)),
            ..ServerSettings::default()
        };
        let relay = fault.map(Relay::new);
        let intercept = relay.clone();
        runtime.spawn(async move {
            let app = skald_server::app(pool, settings);
            if let Some(intercept) = intercept {
                intercept.serve(listener, app).await;
            } else {
                axum::serve(listener, app).await.unwrap();
            }
        });
        Self {
            runtime: Some(runtime),
            db,
            address,
            github,
            relay,
        }
    }

    fn prepare(&self, settings: Settings) -> Delivery {
        let ready = Delivery::prepare(self.address, &self.github, settings).unwrap();
        if let Some(relay) = &self.relay {
            relay.arm();
        }
        ready
    }

    fn discard_pushes_to(&self, recipient: skald_proto::DeviceId) {
        // Deleting each new entry in its insertion transaction avoids racing the live reader.
        // This hook exists only in the fresh test database and is installed after bootstrap.
        let device = recipient.to_string();
        self.db
            .query(async |conn| {
                sqlx::query("CREATE TABLE fixture_discard (device uuid PRIMARY KEY)")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("INSERT INTO fixture_discard VALUES ($1::uuid)")
                    .bind(device)
                    .execute(&mut *conn)
                    .await?;
                sqlx::raw_sql(
                    "
                CREATE FUNCTION fixture_drop_entry() RETURNS trigger LANGUAGE plpgsql AS $$
                BEGIN
                    IF EXISTS (SELECT 1 FROM fixture_discard WHERE device = NEW.device_id) THEN
                        DELETE FROM entries WHERE device_id = NEW.device_id AND seq = NEW.seq;
                    END IF;
                    RETURN NULL;
                END;
                $$;
                CREATE TRIGGER fixture_drop_entry AFTER INSERT ON entries
                FOR EACH ROW EXECUTE FUNCTION fixture_drop_entry()
            ",
                )
                .execute(conn)
                .await?;
                Ok(())
            })
            .unwrap();
    }

    fn remaining(&self) -> i64 {
        self.db
            .query(async |conn| {
                sqlx::query_scalar("SELECT count(*) FROM entries")
                    .fetch_one(conn)
                    .await
            })
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Shut down serving before Database::drop removes its connections.
        self.runtime
            .take()
            .unwrap()
            .shutdown_timeout(Duration::from_secs(1));
    }
}

fn settings() -> Settings {
    Settings {
        devices: 3,
        rates: vec![1, 2],
        seconds: 1,
        text_bytes: 32,
        max_p99_push: Duration::from_secs(1),
        min_history_per_second: 0.0,
    }
}

#[test]
fn steps_report_rates_and_every_delivery_and_history_entry() {
    let fixture = Fixture::start();
    let mut settings = settings();
    settings.devices = 4;
    let report = fixture.prepare(settings).run().unwrap();
    assert_eq!(report.first_failed_check, None, "{report:#?}");
    assert_eq!(report.maximum_passed_rate, Some(2));
    assert_eq!(report.steps.len(), 2);
    for (step, rate) in report.steps.iter().zip([1, 2]) {
        assert_eq!(step.offered_messages_per_second, rate);
        assert_eq!(step.attempted, rate as usize);
        assert_eq!(step.stored, rate as usize);
        assert_eq!(step.history_stored, rate as usize);
        assert_eq!(step.expected_pushes, rate as usize * 3);
        assert_eq!(step.delivered, step.expected_pushes);
        assert_eq!(step.acknowledged, step.expected_pushes);
        assert!(
            step.stored_messages_per_second > 0.0 && step.stored_messages_per_second <= rate as f64
        );
        assert!(
            step.history_entries_per_second > 0.0 && step.history_entries_per_second <= rate as f64
        );
        let latency = step.push_latency_ms.as_ref().expect("latency sample");
        assert!((0.0..=latency.p95).contains(&latency.p50));
        assert!((latency.p95..=1_000.0).contains(&latency.p99));
        assert!(step.errors.is_empty(), "{step:#?}");
    }
    assert_eq!(
        fixture.remaining(),
        0,
        "recipient acks remove mailbox entries"
    );
}

#[test]
fn actual_mailbox_deletion_identifies_the_lost_send_and_recipient() {
    let fixture = Fixture::start();
    let run = fixture.prepare(settings());
    let recipient = run.devices().last().unwrap();
    fixture.discard_pushes_to(recipient);
    let report = run.run().unwrap();
    assert_eq!(report.maximum_passed_rate, None, "{report:#?}");
    assert_eq!(
        report.first_failed_check,
        Some("delivery_integrity"),
        "{report:#?}"
    );
    let [step] = report.steps.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(step.stored, 1);
    assert_eq!(step.history_stored, 1);
    assert_eq!(step.expected_pushes, 2);
    assert_eq!(step.delivered, 1);
    let [loss] = step.errors.as_slice() else {
        panic!("{report:#?}")
    };
    let (id, device) = loss
        .strip_prefix("lost message ")
        .unwrap()
        .split_once(" for device ")
        .unwrap();
    assert!(id.parse::<skald_proto::MessageId>().is_ok());
    assert_eq!(device, recipient.to_string());
}

#[test]
fn checking_the_history_rate_bound_stops_after_the_first_step() {
    let fixture = Fixture::start();
    let mut settings = settings();
    settings.min_history_per_second = 1.5;
    let report = fixture.prepare(settings).run().unwrap();
    assert_eq!(report.maximum_passed_rate, None);
    assert_eq!(
        report.first_failed_check,
        Some("min_history_entries_per_second")
    );
    let [step] = report.steps.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(step.delivered, step.expected_pushes);
    assert_eq!(step.history_stored, 1);
    assert!(step.errors.is_empty(), "{step:#?}");
}

fn late_push_p99(report: &Report) -> f64 {
    let [step] = report.steps.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(step.delivered, 4);
    assert_eq!(step.acknowledged, 4);
    assert_eq!(step.history_stored, 2);
    assert!(step.errors.is_empty(), "{step:#?}");
    step.push_latency_ms.as_ref().unwrap().p99
}

#[test]
fn a_late_push_within_the_p99_bound_is_delivered_and_confirmed() {
    let fixture = Fixture::start_with(Some(Fault::DelaySecondPush));
    let mut settings = settings();
    settings.rates = vec![2];
    settings.max_p99_push = Duration::from_secs(2);

    let report = fixture.prepare(settings).run().unwrap();
    assert_eq!(report.first_failed_check, None, "{report:#?}");
    assert_eq!(report.maximum_passed_rate, Some(2));
    assert!((1_000.0..=2_000.0).contains(&late_push_p99(&report)));
    assert_eq!(fixture.remaining(), 0);
}

#[test]
fn a_late_push_outside_the_p99_bound_stops_the_next_step() {
    let fixture = Fixture::start_with(Some(Fault::DelaySecondPush));
    let mut settings = settings();
    settings.rates = vec![2, 3];
    settings.max_p99_push = Duration::from_secs(1);

    let report = fixture.prepare(settings).run().unwrap();
    assert_eq!(
        report.first_failed_check,
        Some("max_p99_push_ms"),
        "{report:#?}"
    );
    assert_eq!(report.maximum_passed_rate, None);
    assert!(late_push_p99(&report) > 1_000.0);
}

#[test]
fn a_missing_ack_confirmation_keeps_the_push_count() {
    let fixture = Fixture::start_with(Some(Fault::DropAck));
    let run = fixture.prepare(settings());
    let recipient = run.devices().last().unwrap();
    let report = run.run().unwrap();

    assert_eq!(
        report.first_failed_check,
        Some("acknowledgement"),
        "{report:#?}"
    );
    assert_eq!(report.maximum_passed_rate, None);
    let [step] = report.steps.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(step.delivered, step.expected_pushes);
    assert_eq!(step.acknowledged, 1);
    assert_eq!(step.history_stored, 1);
    let [problem] = step.errors.as_slice() else {
        panic!("{report:#?}")
    };
    let (id, device) = problem
        .strip_prefix("no ACK confirmation for message ")
        .unwrap()
        .split_once(" on device ")
        .unwrap();
    assert!(id.parse::<skald_proto::MessageId>().is_ok());
    assert_eq!(device, recipient.to_string());
    assert_eq!(
        fixture.remaining(),
        1,
        "the unacknowledged mailbox entry remains"
    );
}
