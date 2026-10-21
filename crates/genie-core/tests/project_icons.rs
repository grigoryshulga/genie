use genie_core::server_db::{AVATAR_MAX_BYTES, SERVER_SCHEMA_VERSION, ServerDb};

#[test]
fn project_icons_persist_are_isolated_and_can_be_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let first;
    let png = b"\x89PNG\r\n\x1a\n test icon";
    {
        let db = ServerDb::open(&path).unwrap();
        db.create_project("shop", "Shop", "tracker", None, None).unwrap();
        db.create_project("web", "Web", "tracker", None, None).unwrap();
        assert!(db.project("shop").unwrap().icon.is_none());
        assert!(db.set_project_icon("shop", b"<svg/>").is_err());
        assert!(db.set_project_icon("shop", &vec![0; AVATAR_MAX_BYTES + 1]).is_err());
        assert!(db.set_project_icon("missing", png).is_err());
        first = db.set_project_icon("shop", png).unwrap().icon.unwrap();
        assert!(first.starts_with("/api/projects/shop/icon?v="));
        assert!(db.project("web").unwrap().icon.is_none());
    }
    let db = ServerDb::open(&path).unwrap();
    assert_eq!(db.project("shop").unwrap().icon.as_deref(), Some(first.as_str()));
    assert_eq!(db.project_icon("shop").unwrap(), Some((png.to_vec(), "image/png".into())));
    let next = db.set_project_icon("shop", &[0xff, 0xd8, 0xff, 0xe0]).unwrap().icon.unwrap();
    assert_ne!(first, next);
    assert!(db.clear_project_icon("shop").unwrap().icon.is_none());
    assert!(db.project_icon("shop").unwrap().is_none());
}

#[test]
fn schema_8_projects_gain_optional_icons_without_changing_their_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    {
        let db = ServerDb::open(&path).unwrap();
        db.create_project("shop", "Shop", "tracker", None, None).unwrap();
        db.conn().execute_batch("ALTER TABLE projects DROP COLUMN icon; ALTER TABLE projects DROP COLUMN icon_type; ALTER TABLE projects DROP COLUMN icon_at; UPDATE meta SET value = '8' WHERE key = 'schema';").unwrap();
    }
    let db = ServerDb::open(&path).unwrap();
    let project = db.project("shop").unwrap();
    assert_eq!(project.name, "Shop");
    assert!(project.icon.is_none());
    let version: String = db.conn().query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0)).unwrap();
    assert_eq!(version, SERVER_SCHEMA_VERSION.to_string());
    assert!(db.set_project_icon("shop", b"\x89PNG\r\n\x1a\n icon").unwrap().icon.is_some());
}
