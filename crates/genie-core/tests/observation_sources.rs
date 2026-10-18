//! Server schema 7: observation sources and their sealed secrets (§13.2, §7.3). Each test runs on
//! a temporary `server.db`; messages are compared exactly, as the API and the CLI show them.

use genie_core::GenieError;
use genie_core::observation::sources::NewSource;
use genie_core::observation::{SourceAuth, SourceKind};
use genie_core::repos::NewRepo;
use genie_core::server_db::{SERVER_SCHEMA_VERSION, ServerDb};

const NAME_RULE: &str = "source name must be 1 to 32 lowercase letters, digits, dashes or underscores, starting with a letter or digit";
const MANUAL: &str = "source name manual is reserved for manual evidence; choose another name";
const SCHEME: &str = "url must start with http:// or https://";
const USER: &str = "url must not contain user info; keep credentials in the source's secret";
const QUERY: &str = "url must not contain a query string; the query belongs to each criterion";
const FRAGMENT: &str = "url must not contain a fragment";
const HOST: &str = "url needs a host, for example https://prometheus.example.com";
const SPACES: &str = "url must not contain spaces";
const SENTRY_ORG: &str = "organization is required for a sentry source: the Sentry organization slug";
const TIMEOUT: &str = "timeoutSecs must be between 1 and 120";

fn server() -> (tempfile::TempDir, ServerDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
    db.create_project("shop", "Shop", "tracker", None, None).unwrap();
    db.create_project("web", "Web", "tracker", None, None).unwrap();
    (dir, db)
}

fn prom(name: &str, url: &str) -> NewSource {
    NewSource {
        name: name.into(),
        kind: SourceKind::Prometheus,
        url: url.into(),
        auth: SourceAuth::None,
        organization: None,
        sentry_project: None,
        environment: None,
        field: None,
        dataset: None,
        timeout_secs: None,
    }
}

fn sentry(name: &str, url: &str, organization: Option<&str>) -> NewSource {
    NewSource { kind: SourceKind::Sentry, organization: organization.map(Into::into), ..prom(name, url) }
}

fn invalid<T: std::fmt::Debug>(r: Result<T, GenieError>) -> String {
    match r {
        Err(GenieError::Invalid(m)) => m,
        other => panic!("expected invalid input, got {other:?}"),
    }
}

fn missing<T: std::fmt::Debug>(r: Result<T, GenieError>) -> String {
    match r {
        Err(GenieError::NotFound(m)) => m,
        other => panic!("expected a missing record, got {other:?}"),
    }
}

fn tables(db: &ServerDb) -> Vec<String> {
    let mut stmt =
        db.conn().prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name GLOB 'observation_*' ORDER BY name").unwrap();
    stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn columns(db: &ServerDb, table: &str) -> Vec<String> {
    let mut stmt = db.conn().prepare("SELECT name FROM pragma_table_info(?1)").unwrap();
    stmt.query_map([table], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn recorded_version(db: &ServerDb) -> String {
    db.conn().query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0)).unwrap()
}

#[test]
fn schema_7_adds_the_two_source_tables_and_is_recorded() {
    let (_dir, db) = server();
    assert_eq!(SERVER_SCHEMA_VERSION, 7);
    assert_eq!(recorded_version(&db), "7");
    assert_eq!(tables(&db), ["observation_secrets", "observation_sources"]);
    assert_eq!(columns(&db, "observation_sources"), ["project", "name", "kind", "url", "config", "created", "updated"]);
    assert_eq!(columns(&db, "observation_secrets"), ["project", "source", "sealed", "hint", "updated"]);
    let unknown_kind =
        db.conn().execute("INSERT INTO observation_sources VALUES ('shop', 'x', 'graphite', 'https://h', '{}', 't', 't')", []);
    assert!(unknown_kind.is_err(), "the table refuses a kind outside prometheus and sentry");
}

#[test]
fn a_schema_6_server_database_gains_the_source_tables_and_keeps_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    {
        let db = ServerDb::open(&path).unwrap();
        db.create_project("shop", "Shop", "tracker", None, None).unwrap();
        db.conn()
            .execute_batch(
                "DROP TABLE observation_secrets; DROP TABLE observation_sources; UPDATE meta SET value = '6' WHERE key = 'schema';",
            )
            .unwrap();
    }
    let db = ServerDb::open(&path).unwrap();
    assert_eq!(recorded_version(&db), "7");
    assert_eq!(tables(&db), ["observation_secrets", "observation_sources"]);
    assert!(db.project_opt("shop").unwrap().is_some(), "the projects survive the upgrade");
    db.add_observation_source("shop", prom("prom", "https://prometheus.example.com")).unwrap();
}

#[test]
fn invalid_sources_are_refused_with_the_rule_they_break() {
    let (_dir, db) = server();
    let cases: Vec<(NewSource, String)> = vec![
        (prom("manual", "https://prometheus.example.com"), MANUAL.into()),
        (prom("Prom", "https://prometheus.example.com"), NAME_RULE.into()),
        (prom("-prom", "https://prometheus.example.com"), NAME_RULE.into()),
        (prom(&"a".repeat(33), "https://prometheus.example.com"), NAME_RULE.into()),
        (prom("prom", "ftp://prometheus.example.com"), SCHEME.into()),
        (prom("prom", "prometheus.example.com"), SCHEME.into()),
        (prom("prom", "https://user:pass@prometheus.example.com"), USER.into()),
        (prom("prom", "https://prometheus.example.com?query=1"), QUERY.into()),
        (prom("prom", "https://prometheus.example.com#top"), FRAGMENT.into()),
        (prom("prom", "https://"), HOST.into()),
        (prom("prom", "https://:9090"), HOST.into()),
        (prom("prom", "https://prometheus example.com"), SPACES.into()),
        (sentry("errors", "https://sentry.example.com", None), SENTRY_ORG.into()),
        (sentry("errors", "https://sentry.example.com", Some("   ")), SENTRY_ORG.into()),
        (NewSource { timeout_secs: Some(0), ..prom("prom", "https://prometheus.example.com") }, TIMEOUT.into()),
        (NewSource { timeout_secs: Some(121), ..prom("prom", "https://prometheus.example.com") }, TIMEOUT.into()),
        (prom("Bad", "ftp://x"), format!("{NAME_RULE}; {SCHEME}")),
    ];
    for (source, expected) in cases {
        let name = source.name.clone();
        assert_eq!(invalid(db.add_observation_source("shop", source)), expected, "source {name:?}");
    }
    assert!(db.observation_sources("shop").unwrap().is_empty(), "a refused source is not stored");
}

#[test]
fn valid_sources_are_stored_in_canonical_form() {
    let (_dir, db) = server();
    let longest = "a".repeat(32);
    let added = db.add_observation_source("shop", prom("2fast_1-x", "HTTPS://Prom.Example.com//")).unwrap();
    assert_eq!(added.url, "https://Prom.Example.com");
    let local = db.add_observation_source("shop", prom(&longest, "  http://localhost:9090/prometheus/ ")).unwrap();
    assert_eq!(local.url, "http://localhost:9090/prometheus");

    let read = db.observation_source("shop", "2fast_1-x").unwrap().unwrap();
    assert_eq!(read.url, "https://Prom.Example.com");
    assert_eq!(read.auth, SourceAuth::None);
    assert_eq!(read.created, read.updated);
    assert_eq!(read.created.len(), 24, "RFC 3339 with milliseconds: {}", read.created);
    assert!(read.created.ends_with('Z'), "{}", read.created);
}

#[test]
fn sentry_settings_are_kept_and_blank_text_is_dropped() {
    let (_dir, db) = server();
    let new = NewSource {
        name: "errors".into(),
        kind: SourceKind::Sentry,
        url: "https://sentry.example.com/".into(),
        auth: SourceAuth::Bearer,
        organization: Some(" acme ".into()),
        sentry_project: Some("shop-api".into()),
        environment: Some("  ".into()),
        field: Some("count_unique(user)".into()),
        dataset: Some("errors".into()),
        timeout_secs: Some(45),
    };
    let added = db.add_observation_source("shop", new).unwrap();
    let read = db.observation_source("shop", "errors").unwrap().unwrap();
    for source in [&added, &read] {
        assert_eq!(source.kind, SourceKind::Sentry);
        assert_eq!(source.url, "https://sentry.example.com");
        assert_eq!(source.auth, SourceAuth::Bearer);
        assert_eq!(source.organization.as_deref(), Some("acme"));
        assert_eq!(source.sentry_project.as_deref(), Some("shop-api"));
        assert_eq!(source.environment, None, "blank text counts as absent");
        assert_eq!(source.field.as_deref(), Some("count_unique(user)"));
        assert_eq!(source.dataset.as_deref(), Some("errors"));
        assert_eq!(source.timeout_secs, Some(45));
    }
}

#[test]
fn sources_are_listed_by_name_within_their_project() {
    let (_dir, db) = server();
    db.add_observation_source("shop", prom("b", "https://b.example.com")).unwrap();
    db.add_observation_source("shop", NewSource { timeout_secs: Some(30), ..prom("a", "https://a.example.com") }).unwrap();
    db.add_observation_source("web", prom("a", "https://web.example.com")).unwrap();
    let names = |project: &str| db.observation_sources(project).unwrap().into_iter().map(|s| s.name).collect::<Vec<_>>();
    assert_eq!(names("shop"), ["a", "b"]);
    assert_eq!(names("web"), ["a"]);
    assert_eq!(invalid(db.add_observation_source("shop", prom("a", "https://other.example.com"))), "source a already exists");
    assert_eq!(missing(db.add_observation_source("nope", prom("x", "https://x.example.com"))), "project nope not found");

    let listed = serde_json::to_value(db.observation_sources("shop").unwrap()).unwrap();
    let first = &listed[0];
    assert_eq!(first["name"], "a");
    assert_eq!(first["kind"], "prometheus");
    assert_eq!(first["auth"], "none");
    assert_eq!(first["timeoutSecs"], 30);
    assert!(first.get("organization").is_none(), "unset options are left out");
    assert!(first.get("secret").is_none(), "a listing never carries a secret");
}

#[test]
fn a_source_can_be_updated_and_removed_but_not_renamed() {
    let (_dir, db) = server();
    let original = db.add_observation_source("shop", prom("prom", "https://old.example.com")).unwrap();
    let updated = db
        .update_observation_source("shop", "prom", NewSource { timeout_secs: Some(60), ..prom("prom", "https://new.example.com/") })
        .unwrap();
    assert_eq!(updated.url, "https://new.example.com");
    assert_eq!(updated.created, original.created, "the creation time is kept");
    let read = db.observation_source("shop", "prom").unwrap().unwrap();
    assert_eq!(read.url, "https://new.example.com");
    assert_eq!(read.timeout_secs, Some(60));
    assert_eq!(read.created, original.created);

    assert_eq!(
        invalid(db.update_observation_source("shop", "prom", prom("other", "https://new.example.com"))),
        "source prom cannot be renamed; add other and remove prom instead"
    );
    assert_eq!(invalid(db.update_observation_source("shop", "prom", prom("prom", "ftp://x"))), SCHEME);
    assert_eq!(db.observation_source("shop", "prom").unwrap().unwrap().url, "https://new.example.com", "a refused update changes nothing");
    assert_eq!(missing(db.update_observation_source("shop", "nope", prom("nope", "https://x.example.com"))), "source nope not found");

    db.remove_observation_source("shop", "prom").unwrap();
    assert!(db.observation_source("shop", "prom").unwrap().is_none());
    assert_eq!(missing(db.remove_observation_source("shop", "prom")), "source prom not found");
}

#[test]
fn a_source_body_refuses_unknown_fields() {
    let body =
        serde_json::json!({"name": "prom", "kind": "prometheus", "url": "https://prometheus.example.com", "secret": "sk-x-0123456789"});
    let err = serde_json::from_value::<NewSource>(body).unwrap_err().to_string();
    assert!(err.contains("unknown field `secret`"), "{err}");

    let body = serde_json::json!({"name": "prom", "kind": "prometheus", "url": "https://prometheus.example.com", "timeoutSecs": 30});
    let source = serde_json::from_value::<NewSource>(body).unwrap();
    assert_eq!(source.timeout_secs, Some(30));
}

#[test]
fn a_source_secret_is_sealed_read_back_hinted_and_never_listed() {
    let (_dir, db) = server();
    db.add_observation_source("shop", NewSource { auth: SourceAuth::Bearer, ..prom("prom", "https://prometheus.example.com") }).unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap(), None);
    assert!(db.observation_secret_info("shop", "prom").unwrap().is_none());

    db.set_observation_secret("shop", "prom", "  sk-prom-0123456789  ").unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap().as_deref(), Some("sk-prom-0123456789"));
    let info = db.observation_secret_info("shop", "prom").unwrap().unwrap();
    assert_eq!((info.name.as_str(), info.hint.as_str(), info.unreadable), ("prom", "…6789", false));
    let sealed: String = db.conn().query_row("SELECT sealed FROM observation_secrets", [], |r| r.get(0)).unwrap();
    assert!(!sealed.contains("sk-prom"), "stored sealed, never in the clear");
    let listed = serde_json::to_string(&db.observation_sources("shop").unwrap()).unwrap();
    assert!(!listed.contains("sk-prom") && !listed.contains('…'), "listings carry neither the secret nor its hint: {listed}");

    db.set_observation_secret("shop", "prom", "sk-prom-abcdefghij").unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap().as_deref(), Some("sk-prom-abcdefghij"), "a new value replaces the old");
    assert_eq!(invalid(db.set_observation_secret("shop", "prom", "a\nb")), "the secret must be one line of at most 4096 characters");

    db.set_observation_secret("shop", "prom", "   ").unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap(), None, "a blank value removes it");
    assert!(db.observation_secret_info("shop", "prom").unwrap().is_none());

    db.set_observation_secret("shop", "prom", "sk-prom-0123456789").unwrap();
    db.delete_observation_secret("shop", "prom").unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap(), None);

    assert_eq!(missing(db.set_observation_secret("shop", "nope", "sk-x-0123456789")), "source nope not found");
    assert_eq!(missing(db.delete_observation_secret("shop", "nope")), "source nope not found");
    assert_eq!(db.observation_secret("shop", "nope").unwrap(), None);
}

#[test]
fn removing_a_source_removes_its_secret() {
    let (_dir, db) = server();
    db.add_observation_source("shop", NewSource { auth: SourceAuth::Basic, ..prom("prom", "https://prometheus.example.com") }).unwrap();
    db.set_observation_secret("shop", "prom", "user:password-1234").unwrap();
    db.remove_observation_source("shop", "prom").unwrap();
    let left: i64 = db.conn().query_row("SELECT COUNT(*) FROM observation_secrets", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0, "the secret goes with the source");
    assert_eq!(db.observation_secret("shop", "prom").unwrap(), None);

    db.add_observation_source("shop", prom("prom", "https://prometheus.example.com")).unwrap();
    assert!(db.observation_secret_info("shop", "prom").unwrap().is_none(), "a source added again starts without the old secret");
}

#[test]
fn a_source_secret_does_not_open_in_another_project_or_source() {
    let (_dir, db) = server();
    for project in ["shop", "web"] {
        db.add_observation_source(project, NewSource { auth: SourceAuth::Bearer, ..prom("prom", "https://prometheus.example.com") })
            .unwrap();
    }
    db.add_observation_source("shop", prom("api", "https://api.example.com")).unwrap();
    db.set_observation_secret("shop", "prom", "sk-shop-0123456789").unwrap();
    db.set_observation_secret("web", "prom", "sk-web-0123456789").unwrap();
    assert_eq!(db.observation_secret("web", "prom").unwrap().as_deref(), Some("sk-web-0123456789"));

    // Moved to another source of the same project, the sealed value does not open.
    db.conn().execute("UPDATE observation_secrets SET source = 'api' WHERE project = 'shop'", []).unwrap();
    assert_eq!(db.observation_secret("shop", "api").unwrap(), None);
    assert!(db.observation_secret_info("shop", "api").unwrap().unwrap().unreadable);
    db.conn().execute("UPDATE observation_secrets SET source = 'prom' WHERE project = 'shop'", []).unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap().as_deref(), Some("sk-shop-0123456789"), "moved back, it opens again");

    // Moved to another project, it does not open there either.
    db.delete_observation_secret("web", "prom").unwrap();
    db.conn().execute("UPDATE observation_secrets SET project = 'web' WHERE project = 'shop'", []).unwrap();
    assert_eq!(db.observation_secret("web", "prom").unwrap(), None);
    assert!(db.observation_secret_info("web", "prom").unwrap().unwrap().unreadable);
    assert_eq!(db.observation_secret("shop", "prom").unwrap(), None);
}

#[test]
fn an_observation_secret_does_not_open_as_a_repository_token() {
    let (_dir, db) = server();
    db.add_repo(
        "shop",
        NewRepo { name: "api".into(), host: "gitlab".into(), remote: "acme/api".into(), mount: Some("api".into()), ..Default::default() },
    )
    .unwrap();
    db.set_repo_token("shop", "api", "glpat-0123456789abcdef").unwrap();
    db.add_observation_source("shop", prom("api", "https://api.example.com")).unwrap();
    db.set_observation_secret("shop", "api", "sk-obs-0123456789").unwrap();

    db.conn().execute("UPDATE repo_secrets SET sealed = (SELECT sealed FROM observation_secrets WHERE project = 'shop')", []).unwrap();
    assert_eq!(db.repo_token("shop", "api").unwrap(), None, "the copied value does not open as a repository token");

    db.set_repo_token("shop", "api", "glpat-0123456789abcdef").unwrap();
    db.conn().execute("UPDATE observation_secrets SET sealed = (SELECT sealed FROM repo_secrets WHERE project = 'shop')", []).unwrap();
    assert_eq!(db.observation_secret("shop", "api").unwrap(), None, "the copied token does not open as a source secret");
}

#[test]
fn a_lost_key_makes_source_secrets_unreadable_until_entered_again() {
    let (dir, db) = server();
    db.add_observation_source("shop", prom("prom", "https://prometheus.example.com")).unwrap();
    db.set_observation_secret("shop", "prom", "sk-prom-0123456789").unwrap();
    std::fs::remove_file(dir.path().join("secrets.key")).unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap(), None);
    assert!(db.observation_secret_info("shop", "prom").unwrap().unwrap().unreadable);
    db.set_observation_secret("shop", "prom", "sk-prom-abcdefghij").unwrap();
    assert_eq!(db.observation_secret("shop", "prom").unwrap().as_deref(), Some("sk-prom-abcdefghij"));
}

#[test]
fn source_secrets_count_as_sealed_secrets_for_the_backup_warning() {
    let (_dir, db) = server();
    assert!(!db.has_sealed_secrets().unwrap());
    db.add_observation_source("shop", prom("prom", "https://prometheus.example.com")).unwrap();
    db.set_observation_secret("shop", "prom", "sk-prom-0123456789").unwrap();
    assert!(db.has_sealed_secrets().unwrap());
}
