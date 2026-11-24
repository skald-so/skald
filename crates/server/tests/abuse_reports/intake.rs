use serde_json::json;
use skald_proto::{NonEmpty, Role};

use crate::common::device_flow::{json, text};
use crate::common::devices::assert_taken;
use crate::common::{assert_holds_none, stored_forms};
use crate::fixture::Fixture;
use crate::https::{Https, PROOF};
use crate::reporting::{
    CAPABILITY, LONG_RETENTION, OTHER_REASON, REASON, documented, get, post, receipt, refused,
    report_path, report_rows, selected, settings, sign_in, start, submission, submit,
};
use crate::rosters::{first, member, next};
use crate::sessions::Api;
use crate::tokens::{Stage, bearer};

#[test]
fn only_members_discover_the_configured_reason_categories() {
    let github = 3_305_010_001;
    let mut fixture = Fixture::new(github);
    let carol = sign_in(&fixture, &fixture.team.carol, github + 2);
    let eve = sign_in(&fixture, &fixture.eve, github + 4);
    let server = start(&mut fixture);
    let team = &fixture.team;
    let api = Api::of(&team.flow).on(&server);
    let path = format!("/spaces/{}/abuse-reporting", team.space);

    let response = get(&api, &path, &carol);
    assert_eq!(
        response.status,
        200,
        "reporting capability: {}",
        text(&response)
    );
    documented("get", CAPABILITY, &response);
    let body = json(&response);
    let mut reasons = body["reasons"]
        .as_array()
        .expect("reason categories")
        .clone();
    reasons.sort_by_key(serde_json::Value::to_string);
    assert_eq!(reasons, [json!(REASON), json!(OTHER_REASON)]);

    let blocked = get(&api, &path, &eve);
    crate::deletion::assert_refused(&blocked, "capability for a non-member");
    documented("get", CAPABILITY, &blocked);
    assert!(json(&blocked).get("reasons").is_none());
}

#[test]
fn a_members_report_returns_only_a_durable_receipt_and_retry_keeps_one_record() {
    let github = 3_305_010_011;
    let mut fixture = Fixture::new(github);
    // Dave is an ordinary space member and does not belong to its channel.
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let bob = sign_in(&fixture, &fixture.team.bob, github + 1);
    let config = settings("5", "10", LONG_RETENTION);
    let mut server = fixture.team.flow.stand.restart(&[], &config);
    let team = &fixture.team;
    let (api, revision) = selected(team, &server);
    let body = submission(&team.bob, REASON, &revision);
    let first = receipt(&submit(&api, team.space, &dave, &body), &[201]);
    let rows = report_rows(fixture.db());
    assert_eq!(rows.len(), 1, "one durable report");
    for value in [
        team.dave.account.to_string(),
        team.bob.account.to_string(),
        team.space.to_string(),
        REASON.to_owned(),
        first.clone(),
    ] {
        assert!(
            rows[0].contains(&value),
            "the report stores {value:?}: {rows:?}"
        );
    }
    for value in [
        team.dave.account.to_string(),
        team.bob.account.to_string(),
        team.space.to_string(),
        REASON.to_owned(),
    ] {
        assert!(!first.contains(&value), "the receipt exposes {value:?}");
    }

    server.stop();
    let restarted = team.flow.stand.instance(&[], &config);
    let api = Api::of(&team.flow).on(&restarted);
    let retried = receipt(&submit(&api, team.space, &dave, &body), &[200, 201]);
    assert_eq!(retried, first);
    assert_eq!(
        report_rows(fixture.db()),
        rows,
        "a retry leaves one unchanged record"
    );
    assert!(api.web_works(&bob), "the target's session still works");

    let path = report_path(team.space);
    let listed = get(&api, &path, &dave);
    assert!(
        (400..500).contains(&listed.status),
        "a member cannot list reports: {}",
        text(&listed)
    );
    let mut detail = url::Url::parse(&format!("http://localhost{path}")).expect("a report path");
    detail
        .path_segments_mut()
        .expect("path segments")
        .push(&first);
    let read = get(&api, detail.path(), &dave);
    assert!(
        (400..500).contains(&read.status),
        "a member cannot read report details: {}",
        text(&read)
    );
}

#[test]
fn both_accounts_must_belong_to_the_selected_space_without_revealing_other_accounts() {
    let github = 3_305_010_021;
    let mut fixture = Fixture::new(github);
    let carol = sign_in(&fixture, &fixture.team.carol, github + 2);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let eve = sign_in(&fixture, &fixture.eve, github + 4);
    let former = fixture.team.dave.account;
    fixture.revise(|body| {
        let members: Vec<_> = body
            .members
            .clone()
            .into_vec()
            .into_iter()
            .filter(|member| member.account != former)
            .collect();
        body.members = NonEmpty::from_vec(members).expect("members remain");
    });
    // The refused pairs share a different space.
    let team = &fixture.team;
    let name = "other-report-space";
    let other = team.dave.create_space(name);
    let one = team.dave.sign(&first(other, name, &team.dave));
    assert_taken(&team.dave.session.store(other, &one));
    let mut body = next(&one, &team.dave.laptop);
    for person in [&team.bob, &team.carol, &fixture.eve] {
        body.members.push(member(person, Role::Member));
    }
    let two = team.dave.sign(&body);
    assert_taken(&team.dave.session.store(other, &two));

    let server = start(&mut fixture);
    let team = &fixture.team;
    let (api, revision) = selected(team, &server);

    let mut missing = submission(&fixture.eve, REASON, &revision);
    missing["target"] = json!("no-such-fixture-account");
    let unknown = submit(&api, team.space, &carol, &missing);
    refused(&unknown, "unknown target", &["no-such-fixture-account"]);
    for (target, what) in [
        (&fixture.eve, "target outside the space"),
        (&team.dave, "removed target"),
    ] {
        let response = submit(
            &api,
            team.space,
            &carol,
            &submission(target, REASON, &revision),
        );
        refused(
            &response,
            what,
            &[target.handle.as_str(), &target.account.to_string()],
        );
        assert_eq!(response.status, unknown.status, "{what}");
        assert_eq!(json(&response), json(&unknown), "{what}");
    }
    for (cookie, what) in [
        (&eve, "reporter outside the space"),
        (&dave, "removed reporter"),
    ] {
        let response = submit(
            &api,
            team.space,
            cookie,
            &submission(&team.bob, REASON, &revision),
        );
        refused(
            &response,
            what,
            &[team.bob.handle.as_str(), &team.bob.account.to_string()],
        );
    }
    let own = submit(
        &api,
        team.space,
        &carol,
        &submission(&team.carol, REASON, &revision),
    );
    refused(&own, "a report about the reporter's own account", &[]);
    assert!(report_rows(fixture.db()).is_empty());
}

#[test]
fn a_request_without_a_session_submits_nothing() {
    let mut fixture = Fixture::new(3_305_010_051);
    let server = start(&mut fixture);
    let team = &fixture.team;
    let (api, revision) = selected(team, &server);
    let request = submission(&team.bob, REASON, &revision);
    let no_session = post(&api, team.space, &[("origin", api.public)], &request);
    assert_eq!(
        no_session.status,
        401,
        "a session is required: {}",
        text(&no_session)
    );
    refused(
        &no_session,
        "no session",
        &[team.bob.handle.as_str(), &team.bob.account.to_string()],
    );
    assert!(report_rows(fixture.db()).is_empty());
}

#[test]
fn a_live_service_token_cannot_submit_a_report() {
    let mut stage = Stage::new(3_305_010_031);
    let created = stage.create();
    assert!(stage.works(&created.token), "a valid service token");
    let server = start(&mut stage.f);
    let team = &stage.f.team;
    let (api, revision) = selected(team, &server);
    let request = submission(&team.bob, REASON, &revision);
    let hidden = [team.bob.handle.as_str(), &team.bob.account.to_string()];

    let proof = Https::on_session(&server, &team.flow, &created.token, &stage.bot).proof();
    let service = post(
        &api,
        team.space,
        &[("authorization", &bearer(&created.token)), (PROOF, &proof)],
        &request,
    );
    refused(&service, "a valid service token and device proof", &hidden);
    assert!(report_rows(stage.db()).is_empty());
}

#[test]
fn a_report_accepts_only_configured_reasons_and_does_not_carry_content() {
    let github = 3_305_010_041;
    let mut fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let server = start(&mut fixture);
    let team = &fixture.team;
    let (api, revision) = selected(team, &server);
    let unknown = submit(
        &api,
        team.space,
        &dave,
        &submission(&team.bob, "fixture-unconfigured-reason", &revision),
    );
    refused(&unknown, "an unconfigured reason", &[]);
    assert!(report_rows(fixture.db()).is_empty());

    let markers = [
        "marker-for-fixture-text",
        "marker-for-fixture-attachment",
        "marker-for-fixture-token",
        "marker-for-fixture-key",
    ];
    let mut request = submission(&team.bob, REASON, &revision);
    request["reporter"] = json!(team.carol.account.to_string());
    for (field, marker) in ["text", "attachment", "token", "key"]
        .into_iter()
        .zip(markers)
    {
        request[field] = json!(marker);
    }
    let response = submit(&api, team.space, &dave, &request);
    let rows = report_rows(fixture.db());
    if response.status == 201 {
        receipt(&response, &[201]);
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].contains(&team.dave.account.to_string()),
            "the session's account is the reporter"
        );
        assert!(
            !rows[0].contains(&team.carol.account.to_string()),
            "the submitted reporter must have no effect"
        );
    } else {
        refused(&response, "extra report fields", &markers);
        assert!(rows.is_empty(), "a refused report leaves no record");
    }
    let forms = markers
        .iter()
        .flat_map(|marker| stored_forms(marker))
        .collect::<Vec<_>>();
    assert_holds_none(&fixture.db().rows(), &forms);
}
