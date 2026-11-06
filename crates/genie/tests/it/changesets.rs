//! Documentation changesets: branches, read-your-writes, publication, merges, rollback and review.

use crate::common::Harness;

use std::path::Path;

use genie::changesets::{self, Editor, Published};
use genie::state::App;
use genie::tasks::{Caller, CreateBody, Kind, StatusBody};
use genie_core::doc_ops::DocOp;
use genie_core::vault::AuthorKind;
use genie_core::work::NewJob;
use genie_core::{Actor, Role, Status};
use serde_json::json;

fn harness() -> Harness {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    h
}

fn vault(h: &Harness) -> std::path::PathBuf {
    h.app.with_vault(|v| Ok(v.root().to_path_buf())).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn head(h: &Harness) -> String {
    git(&vault(h), &["rev-parse", "HEAD"])
}

fn disk(h: &Harness, path: &str) -> String {
    std::fs::read_to_string(vault(h).join(path)).unwrap()
}

fn system() -> Editor {
    Editor {
        name: "genie".into(),
        login: "genie".into(),
        kind: AuthorKind::System,
        task: None,
        project: "shop".into(),
        admin: true,
        role: None,
    }
}

fn person(login: &str) -> Editor {
    Editor {
        name: login.to_string(),
        login: login.to_string(),
        kind: AuthorKind::Human,
        task: None,
        project: "shop".into(),
        admin: false,
        role: None,
    }
}

fn agent(login: &str, task: Option<&str>) -> Editor {
    Editor {
        name: login.to_string(),
        login: login.to_string(),
        kind: AuthorKind::Agent,
        task: task.map(str::to_string),
        project: "shop".into(),
        admin: false,
        role: None,
    }
}

fn write(body: &str) -> DocOp {
    DocOp::WritePage { content: body.to_string() }
}

fn replace(heading: &str, body: &str) -> DocOp {
    DocOp::ReplaceSection { heading: heading.to_string(), body: body.to_string() }
}

/// A page on `main` written by the server (which publishes at once), returning its commit.
fn seed(h: &Harness, path: &str, content: &str) -> String {
    let ed = system();
    let edited = changesets::edit(&h.app, &ed, path, &[write(content)], "seed", None).unwrap();
    match changesets::publish(&h.app, edited.changeset.id, &ed).unwrap() {
        Published::Published { commit, .. } => commit,
        Published::Merging { merge, .. } => panic!("the seed conflicted: {merge:?}"),
    }
}

fn open_edit(h: &Harness, ed: &Editor, path: &str, ops: &[DocOp], note: &str) -> changesets::Edited {
    changesets::edit(&h.app, ed, path, ops, note, None).unwrap()
}

fn publish(h: &Harness, id: i64, ed: &Editor) -> Published {
    changesets::publish(&h.app, id, ed).unwrap()
}

fn merge_job(app: &App, merge: &genie_core::changesets::DocMerge) -> genie_core::work::Job {
    app.with_server(|db| db.job(merge.job.expect("the merge names its job"))).unwrap()
}

/// Two changesets that change the same line of one page; the second publishes into a merge.
fn conflict(h: &Harness) -> (String, changesets::Edited, changesets::Edited, i64) {
    let path = "shop/guide.md";
    seed(h, path, "# Guide\n\n## Body\n\nthe line\n");
    let anna = person("anna");
    let bob = person("bob");
    let first = open_edit(h, &anna, path, &[replace("Body", "anna's line")], "anna");
    let second = open_edit(h, &bob, path, &[replace("Body", "bob's line")], "bob");
    publish(h, first.changeset.id, &anna);
    let merging = publish(h, second.changeset.id, &bob);
    let merge = match merging {
        Published::Merging { merge, .. } => merge.id,
        Published::Published { commit, .. } => panic!("expected a merge, published {commit}"),
    };
    (path.to_string(), first, second, merge)
}

#[test]
fn an_edit_opens_a_branch_and_leaves_main_alone() {
    let h = harness();
    let path = "shop/guide.md";
    let base = seed(&h, path, "# Guide\n\n## Intro\n\nold\n");
    let anna = person("anna");
    let edited = open_edit(&h, &anna, path, &[replace("Intro", "new")], "rewrite the intro");

    assert_eq!(head(&h), base, "main does not move before publication");
    assert_eq!(edited.changeset.status, genie_core::changesets::ChangesetStatus::Open);
    assert_eq!(edited.changeset.branch, format!("changes/{}", edited.changeset.id));
    assert_eq!(edited.changeset.paths, vec![path.to_string()]);
    assert_eq!(edited.changeset.author, "anna");
    let branch = git(&vault(&h), &["show", &format!("{}:{path}", edited.changeset.branch)]);
    assert!(branch.contains("new"), "{branch}");
    let main = git(&vault(&h), &["show", &format!("HEAD:{path}")]);
    assert!(main.contains("old") && !main.contains("new"), "{main}");

    let mine = changesets::read_for(&h.app, &anna, path, None, None, None, None).unwrap();
    assert!(mine["content"].as_str().unwrap().contains("new"), "{mine}");
    assert_eq!(mine["changeset"], json!(edited.changeset.id));
    assert_eq!(mine["version"], json!(edited.commit), "the branch tip is the version read");

    let other = changesets::read_for(&h.app, &person("bob"), path, None, None, None, None).unwrap();
    assert!(other["content"].as_str().unwrap().contains("old"), "{other}");
    assert_eq!(other["changeset"], json!(null));
    assert_eq!(other["openChangesets"][0]["id"], json!(edited.changeset.id));
    assert_eq!(other["openChangesets"][0]["author"], json!("anna"));
}

#[test]
fn publishing_moves_the_file_onto_disk_and_into_the_index() {
    let h = harness();
    let path = "shop/guide.md";
    seed(&h, path, "# Guide\n\n## Intro\n\nold\n");
    let anna = person("anna");
    let edited = open_edit(&h, &anna, path, &[replace("Intro", "new")], "rewrite");
    let Published::Published { commit, changeset } = publish(&h, edited.changeset.id, &anna) else { panic!("a clean publication") };
    assert_eq!(changeset.status, genie_core::changesets::ChangesetStatus::Published);
    assert_eq!(changeset.published_commit.as_deref(), Some(commit.as_str()));
    assert_eq!(head(&h), commit);
    assert!(disk(&h, path).contains("new"), "the working tree is updated");
    let read = h.app.with_vault(|v| v.project_read("shop", path, None, None)).unwrap();
    assert!(read.content.contains("new"), "the index sees the published text");
    assert!(git(&vault(&h), &["log", "-1", "--format=%s"]).contains(&format!("Publish changeset #{}", edited.changeset.id)));
}

#[test]
fn two_changesets_on_different_sections_publish_cleanly() {
    let h = harness();
    let path = "shop/guide.md";
    seed(&h, path, "# Guide\n\n## A\n\na\n\n## B\n\nb\n");
    let anna = person("anna");
    let bob = person("bob");
    let first = open_edit(&h, &anna, path, &[replace("A", "a2")], "a");
    let second = open_edit(&h, &bob, path, &[replace("B", "b2")], "b");
    assert!(matches!(publish(&h, first.changeset.id, &anna), Published::Published { .. }));
    assert!(matches!(publish(&h, second.changeset.id, &bob), Published::Published { .. }));
    let text = disk(&h, path);
    assert!(text.contains("a2") && text.contains("b2"), "{text}");
}

#[test]
fn the_same_line_becomes_a_pending_merge_and_a_merger_job() {
    let h = harness();
    let (path, _, second, merge_id) = conflict(&h);
    let merge = h.app.with_server(|db| db.doc_merge(merge_id)).unwrap();
    assert_eq!(merge.status, genie_core::changesets::MergeStatus::Running);
    assert_eq!(merge.conflicts, vec![path.clone()]);
    assert_eq!(merge.changeset, second.changeset.id);
    let job = merge_job(&h.app, &merge);
    assert_eq!(job.role, "doc-merger");
    assert_eq!(job.status, "queued");
    assert_eq!(job.task, None);
    assert_eq!(job.inputs["merge"], json!(merge_id));
    assert!(job.goal.contains(&format!("Resolve the pending documentation merge {merge_id}")), "{}", job.goal);
    assert_eq!(h.app.with_server(|db| db.changeset(second.changeset.id)).unwrap().status, genie_core::changesets::ChangesetStatus::Merging);
}

#[test]
fn resolving_a_merge_publishes_and_lands_in_history() {
    let h = harness();
    let (path, _, second, merge_id) = conflict(&h);
    let merger = Editor {
        name: "concord".into(),
        login: "concord".into(),
        kind: AuthorKind::Agent,
        task: None,
        project: "shop".into(),
        admin: false,
        role: Some("doc-merger".into()),
    };
    let merged = "# Guide\n\n## Body\n\nanna's line and bob's line\n";
    let result = changesets::resolve_merge(
        &h.app,
        merge_id,
        &merger,
        &[(path.clone(), Some(merged.to_string()))],
        true,
        false,
        false,
        "kept both edits",
    )
    .unwrap();
    assert_eq!(result["status"], "published", "{result}");
    assert!(disk(&h, &path).contains("anna's line and bob's line"));
    let history = changesets::history(&h.app, "shop", &path, 10).unwrap();
    assert!(history[0].subject.contains(&format!("Publish changeset #{}", second.changeset.id)), "{:?}", history[0]);
    let published = h.app.with_server(|db| db.changeset(second.changeset.id)).unwrap();
    assert_eq!(published.status, genie_core::changesets::ChangesetStatus::Published);
    assert_eq!(h.app.with_server(|db| db.doc_merge(merge_id)).unwrap().status, genie_core::changesets::MergeStatus::Resolved);
    let log = git(&vault(&h), &["log", "-1", "--format=%B"]);
    assert!(log.contains("Genie-Merged-By: doc-merger"), "{log}");
}

#[test]
fn conflict_markers_in_a_resolution_are_refused() {
    let h = harness();
    let (path, _, _, merge_id) = conflict(&h);
    let bad = "# Guide\n\n<<<<<<< ours\ntheirs\n=======\nours\n>>>>>>> theirs\n";
    let err =
        changesets::resolve_merge(&h.app, merge_id, &person("bob"), &[(path, Some(bad.to_string()))], true, false, false, "").unwrap_err();
    assert!(err.to_string().contains("conflict markers"), "{err}");
    assert_ne!(h.app.with_server(|db| db.doc_merge(merge_id)).unwrap().status, genie_core::changesets::MergeStatus::Resolved);
}

#[test]
fn failing_a_merge_hands_it_to_people() {
    let h = harness();
    let (_, _, _, merge_id) = conflict(&h);
    let result = changesets::resolve_merge(&h.app, merge_id, &person("bob"), &[], false, true, false, "cannot tell").unwrap();
    assert_eq!(result["status"], "failed");
    let merge = h.app.with_server(|db| db.doc_merge(merge_id)).unwrap();
    assert_eq!(merge.status, genie_core::changesets::MergeStatus::Failed);
    assert_eq!(merge.error.as_deref(), Some("cannot tell"));
}

#[test]
fn revert_restores_the_previous_content() {
    let h = harness();
    let path = "shop/guide.md";
    seed(&h, path, "# Guide\n\n## Intro\n\nv1\n");
    let anna = Editor { admin: true, ..person("anna") };
    let edited = open_edit(&h, &anna, path, &[replace("Intro", "v2")], "v2");
    let Published::Published { changeset, .. } = publish(&h, edited.changeset.id, &anna) else { panic!("published") };
    assert!(disk(&h, path).contains("v2"));
    let reverted = changesets::revert(&h.app, changeset.id, &anna).unwrap();
    assert!(matches!(reverted, Published::Published { .. }));
    assert!(disk(&h, path).contains("v1"), "{}", disk(&h, path));
    let published = h.app.with_server(|db| db.changeset(changeset.id)).unwrap();
    assert_eq!(published.review_decision, Some(genie_core::changesets::ReviewDecision::Reverted));
    assert_eq!(published.reviewed_by.as_deref(), Some("anna"));
    let reverting = h
        .app
        .with_server(|db| {
            db.changesets(&genie_core::changesets::ChangesetFilter {
                statuses: vec![genie_core::changesets::ChangesetStatus::Published],
                ..Default::default()
            })
        })
        .unwrap()
        .into_iter()
        .find(|c| c.reverts == Some(changeset.id))
        .expect("a published revert changeset");
    assert_eq!(reverting.origin, genie_core::changesets::ChangesetOrigin::Revert);
}

#[test]
fn restore_publishes_a_page_version_from_main() {
    let h = harness();
    let path = "shop/guide.md";
    let first = seed(&h, path, "# Guide\n\n## Intro\n\nv1\n");
    let anna = person("anna");
    let edited = open_edit(&h, &anna, path, &[replace("Intro", "v2")], "v2");
    publish(&h, edited.changeset.id, &anna);
    assert!(disk(&h, path).contains("v2"));
    let restored = changesets::restore(&h.app, &anna, path, &first).unwrap();
    assert!(matches!(restored, Published::Published { .. }));
    assert!(disk(&h, path).contains("v1"));
}

#[test]
fn a_locked_section_refuses_an_agent_edit() {
    let h = harness();
    h.app
        .with_vault(|v| {
            v.config.spaces.get_mut("shop").unwrap().sections.insert(
                "secret".into(),
                genie_core::vault::Section {
                    policy: Some(genie_core::vault::Policy {
                        humans: genie_core::vault::Publish::Direct,
                        agents: genie_core::vault::Publish::Locked,
                    }),
                    owners: vec![],
                },
            );
            v.save_config()
        })
        .unwrap();
    let bot = agent("bot", None);
    let err = changesets::edit(&h.app, &bot, "shop/secret/page.md", &[write("# Secret\n\nx\n")], "", None).unwrap_err();
    assert!(err.to_string().contains("is locked"), "{err}");
    assert!(h.app.with_server(|db| db.changesets(&Default::default())).unwrap().is_empty(), "no changeset is opened");
}

#[test]
fn review_sections_publish_unreviewed_until_an_owner_reviews() {
    let h = harness();
    h.app
        .with_vault(|v| {
            v.config.spaces.get_mut("shop").unwrap().sections.insert(
                "review".into(),
                genie_core::vault::Section {
                    policy: Some(genie_core::vault::Policy {
                        humans: genie_core::vault::Publish::Direct,
                        agents: genie_core::vault::Publish::Review,
                    }),
                    owners: vec!["owner".into()],
                },
            );
            v.save_config()
        })
        .unwrap();
    let bot = agent("bot", None);
    let edited = open_edit(&h, &bot, "shop/review/page.md", &[write("# Page\n\nx\n")], "add");
    let Published::Published { changeset, .. } = publish(&h, edited.changeset.id, &bot) else { panic!("published") };
    assert!(changeset.review_needed);
    let other = person("other");
    let err = changesets::review(&h.app, changeset.id, &other).unwrap_err();
    assert!(err.to_string().contains("owners"), "{err}");
    let owner = person("owner");
    let reviewed = changesets::review(&h.app, changeset.id, &owner).unwrap();
    assert_eq!(reviewed.reviewed_by.as_deref(), Some("owner"));
    assert_eq!(reviewed.review_decision, Some(genie_core::changesets::ReviewDecision::Reviewed));
}

#[test]
fn an_agent_cannot_close_a_task_with_an_open_changeset() {
    let h = harness();
    let anna = Caller::person("shop", "anna");
    let task = genie::tasks::create(
        &h.app,
        &anna,
        CreateBody { title: "Document the API".into(), description: Some("d".into()), ..Default::default() },
    )
    .unwrap();
    let bot = agent("bot", Some(&task.id));
    open_edit(&h, &bot, "shop/api.md", &[write("# API\n\nx\n")], "draft");
    let job = h
        .app
        .with_server(|db| {
            db.create_job(NewJob {
                project: "shop".into(),
                task: Some(task.id.clone()),
                role: "executor".into(),
                goal: "docs".into(),
                workspace: "none".into(),
                ..Default::default()
            })
        })
        .unwrap();
    let caller =
        Caller { project: "shop".into(), actor: Actor::new("bot", Role::Executor), kind: Kind::Agent { team: None, job: Some(job.id) } };
    let err = genie::tasks::set_status(&h.app, &caller, &task.id, StatusBody::to(Status::Done, None)).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("publish or discard documentation changeset #"), "{text}");
}

#[test]
fn the_sweep_retries_a_finished_merger_job_then_fails_the_merge() {
    let h = harness();
    let (_, _, second, merge_id) = conflict(&h);
    let merge = h.app.with_server(|db| db.doc_merge(merge_id)).unwrap();
    let first_job = merge.job.unwrap();
    h.app
        .with_server(|db| {
            db.start_job(first_job)?;
            db.finish_job(first_job, true, None, 3)?;
            Ok(())
        })
        .unwrap();

    changesets::sweep(&h.app).unwrap();
    let retried = h.app.with_server(|db| db.doc_merge(merge_id)).unwrap();
    assert_eq!(retried.status, genie_core::changesets::MergeStatus::Running);
    assert_eq!(retried.attempts, 2);
    assert_ne!(retried.job, Some(first_job), "a new job is queued");
    let second_job = retried.job.unwrap();
    h.app
        .with_server(|db| {
            db.start_job(second_job)?;
            db.finish_job(second_job, true, None, 3)?;
            Ok(())
        })
        .unwrap();

    changesets::sweep(&h.app).unwrap();
    let failed = h.app.with_server(|db| db.doc_merge(merge_id)).unwrap();
    assert_eq!(failed.status, genie_core::changesets::MergeStatus::Failed);
    assert_eq!(failed.changeset, second.changeset.id);
}
