//! Versions remain useful after published page moves, without crossing project roots.

use super::*;

#[test]
fn published_moves_keep_history_read_diff_and_restore_connected() {
    let h = harness();
    let old = "shop/old name.md";
    let middle = "shop/middle.md";
    let current = "shop/archive/current.md";
    let original = seed(&h, old, "# Guide\n\n## Body\n\noriginal text\n");
    let anna = person("anna");
    let first = changesets::move_page(&h.app, &anna, old, middle, "first move", None).unwrap();
    let first_commit = match publish(&h, first.changeset.id, &anna) {
        Published::Published { commit, .. } => commit,
        _ => panic!("move must publish"),
    };
    let edit = open_edit(&h, &anna, middle, &[replace("Body", "updated text")], "update moved page");
    publish(&h, edit.changeset.id, &anna);
    let second = changesets::move_page(&h.app, &anna, middle, current, "second move", None).unwrap();
    publish(&h, second.changeset.id, &anna);
    let latest = head(&h);

    let history = changesets::history(&h.app, "shop", current, 50).unwrap();
    for expected in [&original, &first_commit, &first.commit, &edit.commit, &second.commit, &latest] {
        assert!(history.iter().any(|commit| &commit.sha == expected), "missing {expected} in {history:?}");
    }
    assert_eq!(changesets::history(&h.app, "shop", current, 1).unwrap()[0].sha, latest);
    let previous = changesets::read_for(&h.app, &anna, current, None, Some(&original), None, None).unwrap();
    assert!(previous["content"].as_str().unwrap().contains("original text"), "{previous}");
    assert_eq!(previous["path"], current);
    let now = changesets::read_for(&h.app, &anna, current, Some("main"), None, None, None).unwrap();
    assert!(now["content"].as_str().unwrap().contains("updated text"));
    let diff = changesets::diff(&h.app, "shop", current, &original, &latest).unwrap();
    assert!(diff.contains("-original text") && diff.contains("+updated text"), "{diff}");
    assert!(matches!(changesets::restore(&h.app, &anna, current, &original).unwrap(), Published::Published { .. }));
    assert!(disk(&h, current).contains("original text"));
    assert!(!vault(&h).join(old).exists(), "restore writes the current name");
    assert!(!vault(&h).join(middle).exists());
}

#[test]
fn historical_paths_cannot_escape_the_selected_project() {
    let h = harness();
    h.project("private");
    let mut private_editor = system();
    private_editor.project = "private".into();
    let secret =
        changesets::edit(&h.app, &private_editor, "private/secret.md", &[write("# Secret\n\nprivate history\n")], "secret", None).unwrap();
    changesets::publish(&h.app, secret.changeset.id, &private_editor).unwrap();
    let private_version = head(&h);
    let path = "shop/moved.md";
    git(&vault(&h), &["mv", "private/secret.md", path]);
    git(&vault(&h), &["commit", "-qm", "external cross-project move"]);
    h.app
        .with_vault(|v| {
            v.refresh()?;
            Ok(())
        })
        .unwrap();
    let anna = person("anna");
    assert!(changesets::history(&h.app, "shop", path, 50).is_err());
    assert!(changesets::read_for(&h.app, &anna, path, None, Some(&private_version), None, None).is_err());
    assert!(changesets::diff(&h.app, "shop", path, &private_version, &head(&h)).is_err());
    assert!(changesets::restore(&h.app, &anna, path, &private_version).is_err());
    assert!(changesets::read_for(&h.app, &anna, path, Some("main"), None, None, None).is_ok());
}

#[test]
fn page_versions_reject_unpublished_commits_and_revision_options() {
    let h = harness();
    let path = "shop/guide.md";
    let original = seed(&h, path, "# Guide\n\n## Body\n\noriginal\n");
    let anna = person("anna");
    let edit = open_edit(&h, &anna, path, &[replace("Body", "unpublished")], "draft");
    for invalid in [edit.commit.as_str(), "--all"] {
        assert!(changesets::read_for(&h.app, &anna, path, None, Some(invalid), None, None).is_err());
        assert!(changesets::diff(&h.app, "shop", path, &original, invalid).is_err());
        assert!(changesets::restore(&h.app, &anna, path, invalid).is_err());
    }
    let draft = changesets::read_for(&h.app, &anna, path, Some(&edit.changeset.id.to_string()), None, None, None).unwrap();
    assert!(draft["content"].as_str().unwrap().contains("unpublished"), "explicit changeset reads still work");
}

#[test]
fn rewritten_self_links_keep_identity_across_diverged_publication_and_restore() {
    let h = harness();
    let original = seed(&h, "shop/old.md", "[[old]]\n");
    let anna = person("anna");
    let moved = changesets::move_page(&h.app, &anna, "shop/old.md", "shop/new.md", "move tiny self link", None).unwrap();
    seed(&h, "shop/other.md", "# Unrelated\n");
    let before_publication = head(&h);
    publish(&h, moved.changeset.id, &anna);
    let path = "shop/new.md";
    assert_eq!(disk(&h, path), "[[shop/new]]\n");
    for version in [&original, &before_publication] {
        let old = changesets::read_for(&h.app, &anna, path, None, Some(version), None, None).unwrap();
        assert_eq!(old["content"], "[[old]]\n");
    }
    assert!(changesets::history(&h.app, "shop", path, 50).unwrap().iter().any(|c| c.sha == original));
    let diff = changesets::diff(&h.app, "shop", path, &original, &head(&h)).unwrap();
    assert!(diff.contains("-[[old]]") && diff.contains("+[[shop/new]]"), "{diff}");
    changesets::restore(&h.app, &anna, path, &original).unwrap();
    assert_eq!(disk(&h, path), "[[old]]\n");
}

#[test]
fn restoring_a_deleted_page_keeps_its_original_versions() {
    let h = harness();
    let path = "shop/page.md";
    let original = seed(&h, path, "# Original\n");
    let anna = person("anna");
    let removed = changesets::remove_page(&h.app, &anna, path, "remove", None).unwrap();
    publish(&h, removed.changeset.id, &anna);
    assert!(!vault(&h).join(path).exists());
    changesets::restore(&h.app, &anna, path, &original).unwrap();
    assert_eq!(disk(&h, path), "# Original\n");
    assert!(changesets::history(&h.app, "shop", path, 50).unwrap().iter().any(|c| c.sha == original));
    let old = changesets::read_for(&h.app, &anna, path, None, Some(&original), None, None).unwrap();
    assert_eq!(old["content"], "# Original\n");
    changesets::restore(&h.app, &anna, path, &original).unwrap();
}

#[test]
fn swapping_page_names_preserves_both_histories() {
    let h = harness();
    seed(&h, "shop/one.md", "# First page\n");
    seed(&h, "shop/two.md", "# Second page\n");
    let before = head(&h);
    let anna = person("anna");
    let moved = changesets::move_page(&h.app, &anna, "shop/one.md", "shop/temporary.md", "swap first", None).unwrap();
    changesets::move_page(&h.app, &anna, "shop/two.md", "shop/one.md", "swap second", Some(moved.changeset.id)).unwrap();
    changesets::move_page(&h.app, &anna, "shop/temporary.md", "shop/two.md", "finish swap", Some(moved.changeset.id)).unwrap();
    publish(&h, moved.changeset.id, &anna);
    for (current, expected) in [("shop/one.md", "# Second page\n"), ("shop/two.md", "# First page\n")] {
        assert_eq!(disk(&h, current), expected);
        let old = changesets::read_for(&h.app, &anna, current, None, Some(&before), None, None).unwrap();
        assert_eq!(old["content"], expected, "{current} must follow its former name");
        assert!(changesets::diff(&h.app, "shop", current, &before, &head(&h)).unwrap().is_empty());
    }
}
