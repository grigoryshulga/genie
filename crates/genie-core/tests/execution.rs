use genie_core::{Actor, CreateInput, Role, Tracker};

#[test]
fn pauses_are_durable_inherited_and_only_people_resume_without_erasing_progress() {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    let person = Actor::new("owner", Role::Human);
    let parent = t.create(&person, CreateInput { title: "Parent".into(), ..Default::default() }).unwrap();
    let child = t.create(&person, CreateInput { title: "Child".into(), parent: Some(parent.id.clone()), ..Default::default() }).unwrap();
    let before = t.execution().stamp(&child.id).unwrap();
    let pause = t.execution().pause(&person, &parent.id, "owner", "Pause test").unwrap();
    let event = t.last_event_id().unwrap();
    assert_eq!(t.execution().pause(&person, &parent.id, "budget", "Cannot overwrite first pause").unwrap(), pause);
    assert_eq!(t.last_event_id().unwrap(), event);
    assert_eq!(t.execution().blocking(&child.id).unwrap().unwrap().task, parent.id);
    assert!(t.execution().ensure_running(&child.id).is_err());
    assert!(t.execution().resume(&Actor::new("manager", Role::Orchestrator), &parent.id).is_err());
    drop(t);
    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    assert_eq!(t.execution().state(&parent.id).unwrap(), Some(pause));
    assert_eq!(t.get(&child.id).unwrap().description, child.description);
    t.execution().resume(&person, &parent.id).unwrap();
    t.execution().ensure_running(&child.id).unwrap();
    assert_ne!(t.execution().stamp(&child.id).unwrap(), before, "old prepared workers stay fenced after resume");
    let last = t.last_event_id().unwrap();
    t.execution().resume(&person, &parent.id).unwrap();
    assert_eq!(t.last_event_id().unwrap(), last, "repeated resume is idempotent");
}

#[test]
fn version_eleven_upgrade_adds_execution_control_without_changing_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    let actor = Actor::new("owner", Role::Human);
    let task = t.create(&actor, CreateInput { title: "Retained".into(), ..Default::default() }).unwrap();
    t.conn().execute_batch("DROP TABLE task_execution; UPDATE meta SET value='11' WHERE key='schema';").unwrap();
    drop(t);
    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    assert!(t.execution().state(&task.id).unwrap().is_none());
    assert_eq!(t.get(&task.id).unwrap().title, task.title);
    assert_eq!(t.get(&task.id).unwrap().status, task.status);
    t.execution().pause(&actor, &task.id, "owner", "New control").unwrap();
}
