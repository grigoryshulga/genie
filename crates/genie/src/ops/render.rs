//! Answers of the API as plain text for people and language models, as the
//! command line of the TypeScript version printed them.

use serde_json::{Value, json};

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

const STATUSES: &[&str] =
    &["inbox", "draft", "refining", "ready", "in_progress", "review", "changes_requested", "approved", "needs_owner", "done", "cancelled"];

pub fn status_icon(status: &str) -> &'static str {
    match status {
        "inbox" => "✉",
        "draft" => "·",
        "refining" => "?",
        "ready" => "○",
        "in_progress" => "◐",
        "review" => "◑",
        "changes_requested" => "↺",
        "approved" => "◕",
        "needs_owner" => "!",
        "done" => "●",
        "cancelled" => "✕",
        _ => " ",
    }
}

/// `id — title · status X`
pub fn summary(v: &Value) -> String {
    format!("{} — {} · status {}", s(v, "id"), s(v, "title"), s(v, "status"))
}

/// What a task line says after the title: criteria done, and what needs attention.
fn tail(t: &Value) -> String {
    let mut flags = Vec::new();
    if let Some(q) = t["needsOwner"]["question"].as_str() {
        flags.push(format!("OWNER: {q}"));
    }
    if let Some(r) = t["blocked"]["reason"].as_str() {
        flags.push(format!("BLOCKED: {r}"));
    }
    let open = strs(&t["openDeps"]);
    if !open.is_empty() {
        flags.push(format!("waits {}", open.join(",")));
    }
    if let Some(team) = t["team"].as_str() {
        flags.push(format!("team {team}"));
    }
    if let Some(a) = t["assignee"].as_str() {
        flags.push(format!("@{a}"));
    }
    if s(t, "type") == "epic" {
        flags.push(format!("{}/{} tasks closed", t["childrenClosed"], t["children"]));
    } else if let Some(p) = t["parent"].as_str() {
        flags.push(format!("epic {p}"));
    }
    let total = t["acceptanceTotal"].as_i64().unwrap_or(0);
    let ac = if total > 0 { format!(" [{}/{total}]", t["acceptanceDone"]) } else { String::new() };
    format!(
        "{}{}{ac}{}",
        if s(t, "type") == "epic" { "[epic] " } else { "" },
        s(t, "title"),
        if flags.is_empty() { String::new() } else { format!("  ({})", flags.join("; ")) }
    )
}

/// A task in one line under a status heading of a board.
pub fn one_line(t: &Value) -> String {
    format!("{} {:<6} P{} {:<17} {}", status_icon(s(t, "status")), s(t, "id"), t["priority"], s(t, "status"), tail(t))
}

/// A task in one line of a list: the id first, for people and for scripts (`awk '{print $1}'`).
pub fn list_line(t: &Value) -> String {
    format!("{:<8} {:<18} P{} {}", s(t, "id"), s(t, "status"), t["priority"], tail(t))
}

pub fn list(v: &Value) -> String {
    let rows = v.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return "(no tasks)".into();
    }
    rows.iter().map(list_line).collect::<Vec<_>>().join("\n")
}

/// A task in full, for an agent or a person.
pub fn task(t: &Value, history: bool) -> String {
    let mut out = vec![
        format!("# {}{} — {}", if s(t, "type") == "epic" { "Epic " } else { "" }, s(t, "id"), s(t, "title")),
        format!("type {} · status {} · priority P{}", s(t, "type"), s(t, "status"), t["priority"]),
    ];
    if let Some(p) = t["parent"].as_str() {
        out.push(format!("epic/parent: {p}"));
    }
    if let Some(team) = t["team"].as_str() {
        out.push(format!("team: {team}"));
    }
    if let Some(w) = t["worktree"]["path"].as_str() {
        out.push(format!("worktree: {w}{}", t["worktree"]["branch"].as_str().map(|b| format!(" @ {b}")).unwrap_or_default()));
    }
    let labels = strs(&t["labels"]);
    if !labels.is_empty() {
        out.push(format!("labels: {}", labels.join(", ")));
    }
    if let Some(b) = t["blocked"].as_object() {
        out.push(format!("BLOCKED: {}", b.get("reason").and_then(Value::as_str).unwrap_or_default()));
    }
    if let Some(n) = t["needsOwner"].as_object() {
        out.push(format!("WAITING FOR THE OWNER: {}", n.get("question").and_then(Value::as_str).unwrap_or_default()));
    }
    if !s(t, "mergeStrategy").is_empty() {
        out.push(format!("integration: {}", s(t, "mergeStrategy")));
    }
    if let Some(a) = t["assignee"].as_str() {
        out.push(format!("person responsible: @{a} (mention them in a comment to reach them)"));
    }
    if let Some(e) = t["epic"].as_object() {
        out.push(format!(
            "\n## Epic {}: {}\n{}",
            e["id"].as_str().unwrap_or_default(),
            e["title"].as_str().unwrap_or_default(),
            e["description"].as_str().unwrap_or_default()
        ));
        if let Some(arts) = e["artifacts"].as_array().filter(|a| !a.is_empty()) {
            out.push("Shared artifacts of the epic (read with `genie task artifact-read N --task EPIC`):".into());
            for a in arts {
                out.push(format!("- #{} [{}] {}", a["id"], s(a, "kind"), s(a, "name")));
            }
        }
    }
    let epic = s(t, "type") == "epic";
    out.push(format!(
        "\n## {}\n{}",
        if epic { "Goal" } else { "Description" },
        if s(t, "description").is_empty() { "(empty)" } else { s(t, "description") }
    ));
    if let Some(ac) = t["acceptance"].as_array() {
        out.push(format!(
            "\n## {} ({}/{})",
            if epic { "Success criteria" } else { "Acceptance criteria" },
            ac.iter().filter(|a| a["done"] == json!(true)).count(),
            ac.len()
        ));
        for a in ac {
            out.push(format!("{}. [{}] {}", a["id"], if a["done"] == json!(true) { "x" } else { " " }, s(a, "text")));
        }
    }
    if !s(t, "plan").is_empty() {
        out.push(format!("\n## {}\n{}", if epic { "Roadmap" } else { "Plan" }, s(t, "plan")));
    }
    if !s(t, "notes").is_empty() {
        let notes = s(t, "notes");
        let tail: String = notes.chars().rev().take(3000).collect::<Vec<_>>().into_iter().rev().collect();
        out.push(format!("\n## Notes{}\n{tail}", if notes.len() > tail.len() { " (latest)" } else { "" }));
    }
    let deps = strs(&t["deps"]);
    if !deps.is_empty() {
        out.push(format!("\ndepends on: {}", deps.join(", ")));
    }
    let children = strs(&t["children"]);
    if !children.is_empty() {
        out.push(format!("children: {}", children.join(", ")));
    }
    if let Some(arts) = t["artifacts"].as_array().filter(|a| !a.is_empty()) {
        out.push("\n## Artifacts (read with `genie task artifact-read N`)".into());
        for a in arts {
            out.push(format!("- #{} [{}] {} by {} ({} bytes)", a["id"], s(a, "kind"), s(a, "name"), s(a, "author"), a["size"]));
        }
    }
    if let Some(cs) = t["comments"].as_array().filter(|a| !a.is_empty()) {
        out.push(format!("\n## Comments (latest {} of {})", cs.len().min(15), cs.len()));
        for c in cs.iter().rev().take(15).collect::<Vec<_>>().into_iter().rev() {
            out.push(format!("- [{}] {} ({}): {}", s(c, "kind"), s(c, "author"), s(c, "role"), s(c, "text")));
        }
    }
    if history && let Some(hs) = t["history"].as_array() {
        out.push("\n## History".into());
        for h in hs {
            let change = match (h["from"].as_str(), h["to"].as_str()) {
                (Some(f), Some(to)) => format!(" {f} → {to}"),
                _ => String::new(),
            };
            let note = h["note"].as_str().map(|n| format!(" — {n}")).unwrap_or_default();
            out.push(format!("- {} {} ({}): {}{change}{note}", s(h, "at"), s(h, "actor"), s(h, "role"), s(h, "event")));
        }
    }
    out.join("\n")
}

/// Epics with progress, tasks by status, active teams (`genie task board`).
pub fn board(epics: &Value, tasks: &Value, teams: &Value) -> String {
    let mut lines = Vec::new();
    let epics = epics.as_array().cloned().unwrap_or_default();
    if !epics.is_empty() {
        lines.push(format!("◆ EPICS ({})", epics.len()));
        lines.extend(epics.iter().map(|e| format!("   {}", one_line(e))));
        lines.push(String::new());
    }
    let tasks = tasks.as_array().cloned().unwrap_or_default();
    for st in STATUSES {
        let group: Vec<&Value> = tasks.iter().filter(|t| s(t, "status") == *st).collect();
        if group.is_empty() {
            continue;
        }
        lines.push(format!("{} {} ({})", status_icon(st), st.to_uppercase(), group.len()));
        lines.extend(group.iter().map(|t| format!("   {}", one_line(t))));
    }
    let teams: Vec<Value> = teams.as_array().cloned().unwrap_or_default().into_iter().filter(|t| s(t, "state") == "active").collect();
    if !teams.is_empty() {
        lines.push(String::new());
        lines.push("TEAMS".into());
        for t in &teams {
            let members: Vec<String> = t["members"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|m| format!("{}[{}]", s(m, "name"), s(m, "status")))
                .collect();
            lines.push(format!("   {} → {}: {}", s(t, "id"), s(t, "task"), members.join(" · ")));
        }
    }
    if lines.is_empty() { "the board is empty".into() } else { lines.join("\n") }
}

/// A live session: state, current step, last words.
pub fn session(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let state = v["state"].as_str().unwrap_or("?");
    let since = v["since"].as_str().and_then(|t| t.get(11..19)).unwrap_or("");
    let mut head = format!("session {state} since {since}");
    if let Some(t) = v["tool"].as_object() {
        head.push_str(&format!(
            " · running {}: {}",
            t.get("name").and_then(Value::as_str).unwrap_or("tool"),
            t.get("args").and_then(Value::as_str).unwrap_or_default()
        ));
    }
    if let Some(n) = v["contextTokens"].as_u64() {
        head.push_str(&format!(" · context {}k tokens", n / 1000));
    }
    if v["failures"].as_u64().unwrap_or(0) > 0 {
        head.push_str(&format!(" · {} failed runs ({})", v["failures"], s(v, "lastError")));
    }
    out.push(head);
    if let Some(t) = v["lastThinking"].as_str() {
        out.push(format!("thinking: {t}"));
    }
    if let Some(t) = v["lastText"].as_str() {
        out.push(format!("said: {t}"));
    }
    out
}

pub fn peek(v: &Value) -> String {
    let agent = s(v, "agent");
    if v["session"].is_null() {
        return format!("{agent}: no live session (idle and stopped, or not started yet)");
    }
    let mut out = vec![format!("# {agent}")];
    out.extend(session(&v["session"]));
    if let Some(recent) = v["session"]["recent"].as_array().filter(|r| !r.is_empty()) {
        out.push("\nRecent activity:".into());
        out.extend(recent.iter().filter_map(Value::as_str).map(|l| format!("  {l}")));
    }
    if let Some(conv) = v["conversation"].as_array() {
        out.push("\nLatest conversation:".into());
        for m in conv {
            out.push(format!("[{}] {}", m["role"].as_str().unwrap_or("?"), s(m, "text")));
        }
    }
    out.join("\n")
}

/// Every agent of the project (`genie team board`).
pub fn agents(v: &Value) -> String {
    let mut out = vec![format!("agents ({} mode)", v["mode"].as_str().unwrap_or("?"))];
    for a in v["agents"].as_array().cloned().unwrap_or_default() {
        let mut line = format!("- {} ({})", s(&a, "agent"), s(&a, "role"));
        if let Some(state) = a["state"].as_str().filter(|x| *x != "active") {
            line.push_str(&format!(" · {state}"));
        }
        let pending = a["pending"].as_u64().unwrap_or(0);
        if pending > 0 {
            line.push_str(&format!(" · {pending} waiting"));
        }
        if let Some(st) = a["status"].as_str().filter(|x| !x.is_empty()) {
            line.push_str(&format!(" · \"{st}\""));
        }
        out.push(line);
        if a["session"].is_object() {
            out.extend(session(&a["session"]).into_iter().map(|l| format!("    {l}")));
        }
    }
    out.join("\n")
}

/// A team: roster and recent mail.
pub fn team(v: &Value, mail: usize) -> String {
    let mut out = vec![format!("team {} · task {} · {}", s(v, "id"), s(v, "task"), s(v, "state"))];
    if let Some(w) = v["worktree"].as_object() {
        out.push(format!(
            "worktree {} @ {}",
            w.get("path").and_then(Value::as_str).unwrap_or_default(),
            w.get("branch").and_then(Value::as_str).unwrap_or_default()
        ));
    }
    for m in v["members"].as_array().cloned().unwrap_or_default() {
        out.push(format!("- {} ({}) · {} · {}", s(&m, "name"), s(&m, "role"), s(&m, "activity"), s(&m, "status")));
    }
    let mails = v["mail"].as_array().cloned().unwrap_or_default();
    if mail > 0 && !mails.is_empty() {
        out.push("\nRecent mail:".into());
        for m in mails.iter().rev().take(mail).collect::<Vec<_>>().into_iter().rev() {
            let text: String = s(m, "text").chars().take(300).collect();
            out.push(format!("- {} → {}: {text}", s(m, "from"), s(m, "to")));
        }
    }
    out.join("\n")
}

pub fn teams(v: &Value) -> String {
    let rows = v.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return "no teams".into();
    }
    rows.iter()
        .map(|t| {
            let members: Vec<String> = t["members"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|m| format!("{}/{}", s(m, "name"), m["model"].as_str().unwrap_or("default")))
                .collect();
            format!(
                "{} ({}) task {}{}: {}",
                s(t, "id"),
                s(t, "state"),
                s(t, "task"),
                t["worktree"]["branch"].as_str().map(|b| format!(" @ {b}")).unwrap_or_default(),
                members.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Mail of a team, oldest first.
pub fn mail(mails: &[Value]) -> String {
    if mails.is_empty() {
        return "no mail".into();
    }
    mails
        .iter()
        .map(|m| {
            let at = s(m, "at").get(11..19).unwrap_or_default();
            let intent = m["intent"].as_str().map(|i| format!("/{i}")).unwrap_or_default();
            format!("#{} {at} {} → {} [{}{intent}]: {}", m["id"], s(m, "from"), s(m, "to"), s(m, "level"), s(m, "text"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn doc_markers(p: &Value) -> String {
    let m = doc_marker_list(p);
    if m.is_empty() { String::new() } else { format!(" [{}]", m.join(", ")) }
}

pub fn doc_line(p: &Value) -> String {
    format!(
        "{}  {} ({}, {}){}",
        s(p, "path"),
        s(p, "title"),
        p["type"].as_str().unwrap_or("untyped"),
        p["status"].as_str().unwrap_or("status unknown"),
        doc_markers(p)
    )
}

fn doc_marker_list(p: &Value) -> Vec<String> {
    let mut m = Vec::new();
    if let Some(st) = p["status"].as_str().filter(|x| *x == "draft" || *x == "deprecated") {
        m.push(st.to_string());
    }
    if p["stale"] == json!(true) {
        m.push("stale".into());
    }
    let d = p["diagnostics"].as_array().map(Vec::len).unwrap_or(0);
    if d > 0 {
        m.push(format!("diagnostics:{d}"));
    }
    m
}

/// A changeset in one line of the Changes feed: `#7 open anna G-1 Export (2 pages)`.
pub fn changeset_line(c: &Value) -> String {
    let pages = c["paths"].as_array().map(Vec::len).unwrap_or(0);
    format!("#{} {} {} {} {} ({pages} pages)", c["id"], s(c, "status"), s(c, "author"), c["task"].as_str().unwrap_or("-"), s(c, "title"))
}

/// One commit of a vault history: short SHA, day, author, subject.
pub fn commit_line(c: &Value) -> String {
    let sha = s(c, "sha");
    let short = sha.get(..7).unwrap_or(sha);
    format!("{short} {} {} {}", s(c, "date").get(..10).unwrap_or_default(), s(c, "authorName"), s(c, "subject"))
}

/// A changeset in full: the line, the branch, its commits and the diff.
pub fn changeset_detail(v: &Value) -> String {
    let c = if v["changeset"].is_object() { &v["changeset"] } else { v };
    let mut out = vec![changeset_line(c)];
    out.push(format!("branch {} · base {} · origin {}", s(c, "branch"), s(c, "base"), s(c, "origin")));
    let paths = strs(&c["paths"]);
    if !paths.is_empty() {
        out.push(format!("pages: {}", paths.join(", ")));
    }
    if let Some(by) = c["reviewedBy"].as_str().filter(|b| !b.is_empty()) {
        out.push(format!("reviewed by {by}"));
    }
    if let Some(commits) = v["commits"].as_array() {
        out.push(format!("commits: {}", commits.len()));
        out.extend(commits.iter().map(|m| format!("  {}", commit_line(m))));
    }
    if let Some(diff) = v["diff"].as_str().filter(|d| !d.trim().is_empty()) {
        out.push(String::new());
        out.push(diff.to_string());
    }
    out.join("\n")
}

/// A pending merge: the status, the conflicted pages with both sides and the marked text.
pub fn merge(v: &Value) -> String {
    let m = if v["merge"].is_object() { &v["merge"] } else { v };
    let mut out = vec![format!("merge #{} {} · changeset #{}", m["id"], s(m, "status"), m["changeset"])];
    let conflicts = strs(&m["conflicts"]);
    if !conflicts.is_empty() {
        out.push(format!("conflicted: {}", conflicts.join(", ")));
    }
    if let Some(error) = m["error"].as_str().filter(|e| !e.is_empty()) {
        out.push(format!("error: {error}"));
    }
    if !s(m, "note").is_empty() {
        out.push(format!("note: {}", s(m, "note")));
    }
    if m["contradiction"] == json!(true) {
        out.push("a contradiction: the section owners review it".into());
    }
    for p in v["pages"].as_array().cloned().unwrap_or_default() {
        out.push(String::new());
        out.push(format!("## {}", s(&p, "path")));
        for (key, label) in [("base", "base"), ("ours", "published (ours)"), ("theirs", "changeset (theirs)")] {
            if let Some(text) = p[key].as_str().filter(|t| !t.is_empty()) {
                out.push(format!("\n### {label}\n{text}"));
            }
        }
        for (key, label) in [("ours", "published side"), ("theirs", "changeset side")] {
            let side = &p[key];
            if let Some(author) = side["author"].as_str() {
                out.push(format!(
                    "\n{label}: {author}{}{}",
                    side["task"].as_str().map(|t| format!(" · task {t}")).unwrap_or_default(),
                    side["note"].as_str().filter(|n| !n.is_empty()).map(|n| format!(" — {n}")).unwrap_or_default()
                ));
            }
        }
        if let Some(marked) = p["marked"].as_str().filter(|t| !t.trim().is_empty()) {
            out.push(format!("\n### conflicted regions\n{marked}"));
        }
    }
    out.join("\n")
}

/// The required pages of a role, as the prompt section shows them.
pub fn required(v: &Value) -> String {
    let pages: Vec<crate::context::RequiredPageView> = serde_json::from_value(v["pages"].clone()).unwrap_or_default();
    let missing: Vec<String> = serde_json::from_value(v["missing"].clone()).unwrap_or_default();
    crate::context::required_section(&pages, &missing).unwrap_or_else(|| format!("no pages are required for the role {}", s(v, "role")))
}

/// A page with what genie knows about it, then its text.
pub fn doc(p: &Value) -> String {
    let mut lines = vec![format!("# {}", s(p, "title")), format!("- path: {}", s(p, "path"))];
    lines.push(format!(
        "- type: {}, status: {}",
        p["type"].as_str().unwrap_or("untyped"),
        p["status"].as_str().unwrap_or("status unknown")
    ));
    let markers = doc_marker_list(p);
    if !markers.is_empty() {
        lines.push(format!("- markers: {}", markers.join(", ")));
    }
    if let Some(sm) = p["summary"].as_str().filter(|x| !x.is_empty()) {
        lines.push(format!("- summary: {sm}"));
    }
    if let Some(version) = p["version"].as_str().filter(|x| !x.is_empty()) {
        lines.push(format!("- version: {version}"));
    }
    if let Some(id) = p["changeset"].as_i64().or_else(|| p["changeset"]["id"].as_i64()) {
        lines.push(format!("- changeset: #{id} (yours; others read main)"));
    }
    if let Some(others) = p["openChangesets"].as_array().filter(|a| !a.is_empty()) {
        lines.push(format!("- {} open changeset(s) from others touch this page", others.len()));
    }
    if p["unreviewed"].as_array().is_some_and(|a| !a.is_empty()) {
        lines.push("- published changesets here await the owners' review".into());
    }
    for k in ["tags", "aliases", "related"] {
        let v = strs(&p[k]);
        if !v.is_empty() {
            lines.push(format!("- {k}: {}", v.join(", ")));
        }
    }
    lines.push(format!(
        "- verified: {}, updated: {}",
        p["verified"].as_str().unwrap_or("never"),
        p["updated"].as_str().unwrap_or("unknown")
    ));
    for (k, label) in [("staleReasons", "stale reasons"), ("diagnostics", "frontmatter diagnostics")] {
        let v = strs(&p[k]);
        if !v.is_empty() {
            lines.push(format!("- {label} ({}):", v.len()));
            lines.extend(v.iter().map(|r| format!("  - {r}")));
        }
    }
    if let Some(h) = p["heading"].as_str() {
        lines.push(format!("- heading: {h}"));
    }
    let links = p["links"].as_array().cloned().unwrap_or_default();
    if !links.is_empty() {
        lines.push("- links:".into());
        for l in &links {
            let at = l["targetPath"].as_str().map(|t| format!(" ({t})")).unwrap_or_default();
            lines.push(format!("  - {} -> {}{at}", s(l, "target"), s(l, "resolution")));
        }
    }
    let backlinks = strs(&p["backlinks"]);
    if !backlinks.is_empty() {
        lines.push(format!("- backlinks: {}", backlinks.join(", ")));
    }
    if p["truncated"] == json!(true) {
        lines.push("- truncated: pass a larger --max-chars or a --heading for the rest".into());
    }
    lines.push(String::new());
    lines.push(s(p, "content").to_string());
    lines.join("\n")
}

/// Pages a task may have made stale (`genie docs impact`).
pub fn impact(v: &Value) -> String {
    let mut lines = vec!["## Docs impact (non-blocking)".to_string()];
    let cands = v["candidates"].as_array().cloned().unwrap_or_default();
    if cands.is_empty() {
        lines.push("no documentation page looks affected".into());
    }
    for c in &cands {
        lines.push(format!(
            "- {} — {} ({}, {}){}",
            s(c, "path"),
            s(c, "title"),
            c["type"].as_str().unwrap_or("untyped"),
            c["status"].as_str().unwrap_or("status unknown"),
            doc_markers(c)
        ));
        if !s(c, "summary").is_empty() {
            lines.push(format!("    {}", s(c, "summary")));
        }
    }
    let available = v["changedPathsAvailable"] == json!(true);
    for n in strs(&v["notes"]) {
        lines.push(if available { format!("note: {n}") } else { format!("changed paths unavailable: {n}") });
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_line_carries_what_needs_attention() {
        let t = json!({ "id": "G-7", "title": "Export", "type": "task", "status": "needs_owner", "priority": 1,
            "needsOwner": { "question": "CSV?" }, "openDeps": ["G-3"], "parent": "G-1", "assignee": "anna", "acceptanceDone": 1, "acceptanceTotal": 2 });
        assert_eq!(one_line(&t), "! G-7    P1 needs_owner       Export [1/2]  (OWNER: CSV?; waits G-3; @anna; epic G-1)");
        let e = json!({ "id": "G-1", "title": "Returns", "type": "epic", "status": "in_progress", "priority": 2, "children": 4, "childrenClosed": 1 });
        assert!(one_line(&e).ends_with("[epic] Returns  (1/4 tasks closed)"), "{}", one_line(&e));
        assert_eq!(list_line(&t), "G-7      needs_owner        P1 Export [1/2]  (OWNER: CSV?; waits G-3; @anna; epic G-1)");
    }
}
