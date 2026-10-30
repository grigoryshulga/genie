//! What agents know of the project's knowledge base without asking.
//!
//! - **L0** — every page of the project's space by path, title, type, status,
//!   date and summary, with markers for stale, draft, unverified, deprecated
//!   and broken pages — in the system prompt of agents that read the knowledge
//!   base. Metadata only; past its budget the index is compacted and says so.
//! - **L1** — at the start of a task, up to three pages chosen for it in the
//!   kickoff of its team: pages naming the task or its epic in `related` first,
//!   then pages whose `paths` cover paths the task mentions, then pages whose
//!   title, summary, headings, tags or aliases share its words. Pages are
//!   clipped at section boundaries, with a marker.
//!
//! Budgets are estimator units (about four Latin or two Cyrillic characters),
//! not tokens. Ported from the TypeScript version with its tests.

use std::collections::HashSet;

use genie_core::Task;
use genie_core::vault::{DocPage, REQUIRED_PAGE_LIMIT};

use crate::knowledge::glob_matches;
use crate::state::App;

pub const L0_TARGET_UNITS: usize = 600;
pub const L0_HARD_CAP_UNITS: usize = 1000;
pub const L1_TOTAL_UNITS: usize = 4000;
pub const L1_PAGE_UNITS: usize = 1500;
pub const L1_MAX_PAGES: usize = 3;
const L0_SUMMARY_CLIP: usize = 100;
const MIN_TERM_CHARS: usize = 3;
const MIN_PAGE_BUDGET: usize = 200;

/// Estimator units: characters of Cyrillic count ½, everything else ¼; rounded up.
pub fn units(text: &str) -> usize {
    let (mut latin, mut cyrillic) = (0usize, 0usize);
    for c in text.chars() {
        if matches!(c as u32, 0x0400..=0x052f) {
            cyrillic += 1;
        } else {
            latin += 1;
        }
    }
    (latin + 2 * cyrillic).div_ceil(4)
}

/// Lowercase with ё folded to е, so words match whichever spelling was used.
pub fn normalize(word: &str) -> String {
    word.to_lowercase().replace('ё', "е")
}

/// What to know about a page before reading it.
pub fn markers(page: &DocPage) -> Vec<&'static str> {
    let mut out = Vec::new();
    if page.stale {
        out.push("stale");
    }
    if page.status.as_deref() == Some("draft") {
        out.push("draft");
    }
    if page.verified.is_none() {
        out.push("unverified");
    }
    if page.status.as_deref() == Some("deprecated") {
        out.push("deprecated");
    }
    if !page.diagnostics.is_empty() {
        out.push("check frontmatter");
    }
    out
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max { flat } else { format!("{}…", flat.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

// --- L0 --------------------------------------------------------------------------

const L0_HEADER: &str = "## Project knowledge (L0 index)";
const L0_HINT: &str =
    "The pages of the project's knowledge base, metadata only: read one with `genie docs read <path>`, search with `genie docs search`.";

/// The L0 index of some pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L0 {
    pub text: String,
    pub units: usize,
    pub pages: usize,
    pub shown: usize,
    pub compacted: bool,
}

fn l0_line(p: &DocPage) -> String {
    let summary =
        p.summary.as_deref().filter(|s| !s.trim().is_empty()).map(|s| format!(": {}", clip(s, L0_SUMMARY_CLIP))).unwrap_or_default();
    let marks: String = markers(p).iter().map(|m| format!(" [{m}]")).collect();
    format!(
        "- {} — {} ({}, {}, {}){summary}{marks}",
        p.path,
        p.title,
        p.doc_type.as_deref().unwrap_or("untyped"),
        p.status.as_deref().unwrap_or("unknown"),
        p.updated.as_deref().unwrap_or("no date")
    )
}

fn l0_compact(pages: &[DocPage], shown: usize) -> String {
    let draft = pages.iter().filter(|p| p.status.as_deref() == Some("draft")).count();
    let stale = pages.iter().filter(|p| p.stale).count();
    let header = format!("## Project knowledge (L0 index — compact: showing {shown} of {})", pages.len());
    let mut lines: Vec<String> = pages[..shown].iter().map(l0_line).collect();
    lines.push(format!(
        "- … {} more page(s) not listed (compact index: showing {shown} of {}; draft {draft}, stale {stale}). Search them with `genie docs search`.",
        pages.len() - shown,
        pages.len()
    ));
    format!("{header}\n{L0_HINT}\n{}", lines.join("\n"))
}

/// The L0 index of pages in path order; past its target it keeps the longest
/// prefix that fits and marks the rest.
pub fn l0_index(pages: &[DocPage]) -> Option<L0> {
    if pages.is_empty() {
        return None;
    }
    let full = format!("{L0_HEADER}\n{L0_HINT}\n{}", pages.iter().map(l0_line).collect::<Vec<_>>().join("\n"));
    if units(&full) <= L0_TARGET_UNITS {
        return Some(L0 { units: units(&full), text: full, pages: pages.len(), shown: pages.len(), compacted: false });
    }
    let (shown, text) = (0..pages.len())
        .rev()
        .map(|n| (n, l0_compact(pages, n)))
        .find(|(_, t)| units(t) <= L0_TARGET_UNITS)
        .unwrap_or_else(|| (0, l0_compact(pages, 0)));
    Some(L0 { units: units(&text), text, pages: pages.len(), shown, compacted: true })
}

/// The pages of a project's space(s).
fn project_pages(app: &App, project: &str) -> Vec<DocPage> {
    app.with_vault(|v| v.tree()).unwrap_or_default().into_iter().filter(|p| p.project.as_deref() == Some(project)).collect()
}

/// The L0 section of an agent's prompt, when the project has pages.
pub fn l0(app: &App, project: &str) -> Option<String> {
    l0_index(&project_pages(app, project)).map(|l| l.text)
}

// --- required reading -------------------------------------------------------------

const REQUIRED_HEADER: &str = "## Required reading";
const REQUIRED_HINT: &str =
    "Follow these pages of the project's knowledge base. They are complete; versions are listed so you can tell when one changes.";

/// A required page as delivered: its whole body, without frontmatter.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequiredPageView {
    pub path: String,
    pub title: String,
    /// The first 12 characters of the page's content hash.
    pub version: String,
    pub content: String,
    /// Longer than [`REQUIRED_PAGE_LIMIT`]; still delivered whole.
    pub oversize: bool,
}

/// The required pages of a role and the paths that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredReading {
    pub role: String,
    pub pages: Vec<RequiredPageView>,
    pub missing: Vec<String>,
}

/// The Markdown of the required section, or `None` when no page is required.
pub fn required_section(pages: &[RequiredPageView], missing: &[String]) -> Option<String> {
    if pages.is_empty() && missing.is_empty() {
        return None;
    }
    let mut out = format!("{REQUIRED_HEADER}\n\n{REQUIRED_HINT}");
    for p in pages {
        out.push_str(&format!("\n\n### {} (`{}`, version {})\n\n{}", p.title, p.path, p.version, p.content.trim_end()));
    }
    if !missing.is_empty() {
        out.push_str("\n\nMissing required pages:");
        for path in missing {
            out.push_str(&format!("\n- {path}: missing"));
        }
    }
    Some(out)
}

/// Read the pages the project requires for a role: whole bodies, with versions,
/// and the paths that are missing. `None` when the role reads nothing.
pub fn required_reading(app: &App, project: &str, role: &str) -> Option<RequiredReading> {
    let paths = app.with_vault(|v| Ok(v.required_for(project, role))).ok()?;
    if paths.is_empty() {
        return None;
    }
    let mut pages = Vec::new();
    let mut missing = Vec::new();
    for path in paths {
        match app.with_vault(|v| v.read(&path, None, None)) {
            Ok(r) => pages.push(RequiredPageView {
                version: r.page.content_hash.chars().take(12).collect(),
                oversize: r.content.chars().count() > REQUIRED_PAGE_LIMIT,
                path,
                title: r.page.title,
                content: r.content,
            }),
            Err(_) => missing.push(path),
        }
    }
    Some(RequiredReading { role: role.into(), pages, missing })
}

/// The required-reading section of an agent's prompt.
pub fn required(app: &App, project: &str, role: &str) -> Option<String> {
    let reading = required_reading(app, project, role)?;
    required_section(&reading.pages, &reading.missing)
}

/// Roles that must read `path` in a project (`all` stays as it is).
pub fn roles_requiring(app: &App, project: &str, path: &str) -> Vec<String> {
    app.with_vault(|v| {
        Ok(v.config
            .spaces
            .values()
            .filter(|s| s.project.as_deref() == Some(project))
            .flat_map(|s| s.required.iter())
            .filter(|r| r.page == path)
            .flat_map(|r| r.roles.iter().map(|role| role.to_lowercase()))
            .collect())
    })
    .unwrap_or_default()
}

/// The notice a member gets when one of its required pages changed on main.
pub fn required_change_notice(path: &str, changeset: i64, diff: &str) -> String {
    const MAX: usize = 6000;
    let shown: String = diff.chars().take(MAX).collect();
    let cut = if diff.chars().count() > MAX { "\n… (diff cut; read the page with `genie docs read`)" } else { "" };
    format!(
        "Required page `{path}` changed (changeset #{changeset}). Follow the new version from now on; the change:\n\n```diff\n{shown}{cut}\n```"
    )
}

/// Tell the members of running teams whose role must read `path` that it changed on main.
pub fn notify_required_changed(app: &App, project: &str, path: &str, changeset: i64, diff: &str) {
    let roles = roles_requiring(app, project, path);
    if roles.is_empty() {
        return;
    }
    let text = required_change_notice(path, changeset, diff);
    let topic = format!("required:{path}");
    let _ = app.with_tracker(project, |t| {
        for team in t.bus().list(false)? {
            for m in team.members.iter().filter(|m| roles.iter().any(|r| r == "all" || *r == m.role.to_lowercase())) {
                let _ = t.bus().send(genie_core::team::SendMail {
                    team: &team.id,
                    from: "genie",
                    from_role: "system",
                    to: &m.name,
                    text: &text,
                    level: Some("normal"),
                    kind: "message",
                    topic: Some(&topic),
                    ..Default::default()
                });
            }
        }
        Ok(())
    });
}

// --- L1 --------------------------------------------------------------------------

const L1_HEADER: &str = "## Project knowledge (L1 context)";
const L1_HINT: &str =
    "Chosen for this task; stale and draft pages are marked. Read more with `genie docs read <path>` and `genie docs search`.";

/// Words with no topic of their own (English and Russian). A language-level
/// list, never a per-page one; the project's own name is damped on top.
const FUNCTION_WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "from",
    "into",
    "with",
    "without",
    "over",
    "under",
    "about",
    "after",
    "before",
    "between",
    "through",
    "this",
    "that",
    "these",
    "those",
    "they",
    "them",
    "their",
    "there",
    "you",
    "your",
    "its",
    "his",
    "her",
    "our",
    "who",
    "which",
    "what",
    "are",
    "was",
    "were",
    "been",
    "has",
    "have",
    "had",
    "does",
    "did",
    "can",
    "could",
    "may",
    "might",
    "must",
    "should",
    "will",
    "would",
    "not",
    "only",
    "also",
    "then",
    "than",
    "when",
    "where",
    "while",
    "how",
    "all",
    "any",
    "some",
    "more",
    "most",
    "other",
    "such",
    "same",
    "или",
    "либо",
    "если",
    "чтобы",
    "что",
    "как",
    "чем",
    "для",
    "без",
    "под",
    "над",
    "про",
    "при",
    "через",
    "между",
    "после",
    "перед",
    "кроме",
    "это",
    "этот",
    "эта",
    "эти",
    "тот",
    "его",
    "ее",
    "её",
    "их",
    "они",
    "она",
    "оно",
    "все",
    "где",
    "тут",
    "там",
    "тогда",
    "когда",
    "так",
    "тоже",
    "также",
    "ещё",
    "еще",
    "уже",
    "только",
    "очень",
];

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| w.chars().count() >= MIN_TERM_CHARS)
}

/// Paths a task mentions (`src/export/*.rs`, `docs/api.md`), links left out.
pub fn path_candidates(corpus: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in corpus.split(|c: char| !(c.is_ascii_alphanumeric() || "_.-/*?:".contains(c))) {
        if raw.contains("://") {
            continue;
        }
        for piece in raw.split(':') {
            let t = piece.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
            if t.contains('/') && !t.split('/').any(str::is_empty) && !out.iter().any(|x| x == t) {
                out.push(t.to_string());
            }
        }
    }
    out
}

/// The words of a page L1 matches against: title, summary, headings, tags and
/// aliases — never the body, so the choice stays cheap and predictable.
pub fn term_surface(p: &DocPage) -> String {
    let mut parts = vec![p.title.clone(), p.summary.clone().unwrap_or_default()];
    parts.extend(p.headings.iter().cloned());
    parts.extend(p.tags.iter().cloned());
    parts.extend(p.aliases.iter().cloned());
    parts.join("\n")
}

fn surface_words(p: &DocPage) -> HashSet<String> {
    words(&term_surface(p)).map(normalize).collect()
}

/// A task as L1 reads it.
pub struct L1Task<'a> {
    pub id: &'a str,
    /// The epic (or parent task) whose pages count as related too.
    pub epic: Option<&'a str>,
    pub corpus: String,
}

impl<'a> L1Task<'a> {
    pub fn of(task: &'a Task) -> L1Task<'a> {
        let mut corpus = vec![task.title.clone(), task.description.clone(), task.plan.clone()];
        corpus.extend(task.acceptance.iter().map(|a| a.text.clone()));
        L1Task { id: &task.id, epic: task.parent.as_deref(), corpus: corpus.join("\n") }
    }
}

/// A page chosen for a task, with why.
#[derive(Debug, Clone)]
pub struct Chosen {
    pub path: String,
    /// 0: related to the task or its epic; 1: covers a path the task mentions; 2: shares its words.
    pub rank: u8,
    pub reasons: Vec<String>,
    term_matches: usize,
    status_rank: u8,
}

/// Choose up to three pages for a task: by rank, then the most shared words,
/// current before other before draft, then path. Words of the project's own
/// name (`project_words`), function words and words on more than half of the
/// pages choose nothing; deprecated pages come only when related.
pub fn choose(pages: &[DocPage], task: &L1Task, project_words: &[String]) -> Vec<Chosen> {
    let paths = path_candidates(&task.corpus);
    let path_parts: HashSet<&str> = paths.iter().flat_map(|p| p.split(['/', '.'])).collect();
    let mut terms: Vec<&str> = Vec::new();
    for w in words(&task.corpus) {
        if !path_parts.contains(w) && !terms.contains(&w) {
            terms.push(w);
        }
    }
    let surfaces: Vec<HashSet<String>> = pages.iter().map(surface_words).collect();
    let mut damped: HashSet<String> = project_words.iter().flat_map(|n| words(n).map(normalize).collect::<Vec<_>>()).collect();
    damped.extend(FUNCTION_WORDS.iter().map(|w| normalize(w)));
    for t in &terms {
        let n = normalize(t);
        let df = surfaces.iter().filter(|s| s.contains(&n)).count();
        if df >= 2 && df * 2 > pages.len() {
            damped.insert(n);
        }
    }
    let terms: Vec<&str> = terms.into_iter().filter(|t| !damped.contains(&normalize(t))).collect();
    let wanted: Vec<String> = std::iter::once(task.id).chain(task.epic).map(|s| s.trim().to_uppercase()).collect();

    let mut chosen: Vec<Chosen> = Vec::new();
    for (p, surface) in pages.iter().zip(&surfaces) {
        let related: HashSet<String> = p.related.iter().map(|r| r.trim().to_uppercase()).collect();
        let mut reasons: Vec<String> = Vec::new();
        for id in &wanted {
            if related.contains(id) && !reasons.contains(&format!("related {id}")) {
                reasons.push(format!("related {id}"));
            }
        }
        let is_related = !reasons.is_empty();
        let patterns = p.paths.clone().unwrap_or_default();
        let by_path: Vec<&String> =
            paths.iter().filter(|c| patterns.iter().any(|pat| glob_matches(pat, c) || glob_matches(c, pat))).collect();
        let by_terms: Vec<&&str> = terms.iter().filter(|t| surface.contains(&normalize(t))).collect();
        if !is_related && by_path.is_empty() && by_terms.is_empty() {
            continue;
        }
        if !is_related && p.status.as_deref() == Some("deprecated") {
            continue;
        }
        let rank = if is_related {
            0
        } else if !by_path.is_empty() {
            1
        } else {
            2
        };
        reasons.extend(by_path.iter().take(2).map(|c| format!("path match {c}")));
        reasons.extend(by_terms.iter().take(3).map(|t| format!("term match {t}")));
        let status_rank = match p.status.as_deref() {
            Some("current") => 0,
            Some("draft") => 2,
            _ => 1,
        };
        chosen.push(Chosen { path: p.path.clone(), rank, reasons, term_matches: by_terms.len(), status_rank });
    }
    chosen.sort_by(|a, b| {
        (a.rank, std::cmp::Reverse(a.term_matches), a.status_rank, &a.path).cmp(&(
            b.rank,
            std::cmp::Reverse(b.term_matches),
            b.status_rank,
            &b.path,
        ))
    });
    chosen.truncate(L1_MAX_PAGES);
    chosen
}

/// A page clipped to a budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clipped {
    pub text: String,
    pub clipped: bool,
    pub sections: usize,
    pub shown: usize,
}

fn sections(body: &str) -> Vec<String> {
    let is_heading = |l: &str| {
        let t = l.trim_start();
        let n = t.chars().take_while(|c| *c == '#').count();
        (1..=6).contains(&n) && t[n..].starts_with(char::is_whitespace)
    };
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in body.lines() {
        if is_heading(line) && cur.iter().any(|l| !l.trim().is_empty()) {
            out.push(cur.join("\n"));
            cur = vec![line];
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    if out.is_empty() { vec![body.to_string()] } else { out }
}

/// Clip a page at section boundaries within `budget` units, with a marker when
/// anything is left out (its units are reserved, so the text never exceeds the budget).
pub fn clip_at_sections(content: &str, budget: usize) -> Clipped {
    let secs = sections(content);
    let total = secs.len();
    let marker = |shown: usize| {
        format!(
            "\n\n[... page clipped at section boundaries: showing {shown} of {total} sections; read the rest with `genie docs read` ...]"
        )
    };
    if units(content) <= budget {
        return Clipped { text: content.to_string(), clipped: false, sections: total, shown: total };
    }
    let inner = budget.saturating_sub(units(&marker(total)));
    let mut kept: Vec<&str> = Vec::new();
    for s in &secs {
        let mut next = kept.clone();
        next.push(s);
        if units(&next.join("\n")) <= inner {
            kept = next;
        } else {
            break;
        }
    }
    if kept.len() == total {
        return Clipped { text: content.to_string(), clipped: false, sections: total, shown: total };
    }
    if kept.is_empty() {
        let mut lines: Vec<&str> = Vec::new();
        for l in secs[0].lines() {
            let mut next = lines.clone();
            next.push(l);
            if units(&next.join("\n")) <= inner {
                lines = next;
            } else {
                break;
            }
        }
        return Clipped { text: format!("{}{}", lines.join("\n"), marker(0)), clipped: true, sections: total, shown: 0 };
    }
    Clipped { text: format!("{}{}", kept.join("\n"), marker(kept.len())), clipped: true, sections: total, shown: kept.len() }
}

/// The L1 section: the chosen pages within their budgets. `read` gives a page's body.
pub fn l1_context(pages: &[DocPage], chosen: &[Chosen], read: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut used = units(&format!("{L1_HEADER}\n{L1_HINT}"));
    for c in chosen {
        let Some(p) = pages.iter().find(|p| p.path == c.path) else { continue };
        // The status names a draft already; staleness is marked on top of it.
        let heading = format!(
            "### {} — {} ({}, {}{}) — {}",
            p.title,
            p.path,
            p.doc_type.as_deref().unwrap_or("untyped"),
            p.status.as_deref().unwrap_or("unknown"),
            if p.stale { ", stale" } else { "" },
            c.reasons.join(" · ")
        );
        let remaining = L1_TOTAL_UNITS.saturating_sub(used + units(&format!("\n{heading}")));
        if remaining < MIN_PAGE_BUDGET {
            break;
        }
        let Some(body) = read(&p.path) else { continue };
        let clip = clip_at_sections(&body, remaining.min(L1_PAGE_UNITS));
        let block = format!("\n{heading}\n{}", clip.text);
        used += units(&block);
        blocks.push(block);
    }
    (!blocks.is_empty()).then(|| format!("{L1_HEADER}\n{L1_HINT}{}", blocks.concat()))
}

/// The L1 context of a task for its team's kickoff, when pages fit it.
pub fn l1(app: &App, project: &str, task: &Task) -> Option<String> {
    let pages = project_pages(app, project);
    if pages.is_empty() {
        return None;
    }
    let mut names = vec![project.to_string()];
    if let Ok(p) = app.with_server(|db| db.project(project)) {
        names.push(p.name);
        names.extend(p.repo.as_deref().and_then(|r| std::path::Path::new(r).file_name()).map(|n| n.to_string_lossy().into_owned()));
    }
    let chosen = choose(&pages, &L1Task::of(task), &names);
    l1_context(&pages, &chosen, |path| app.with_vault(|v| v.read(path, None, Some(100_000))).ok().map(|r| r.content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(path: &str, title: &str) -> DocPage {
        DocPage {
            root_id: "vault".into(),
            path: path.into(),
            title: title.into(),
            doc_type: Some("guide".into()),
            status: Some("current".into()),
            summary: None,
            tags: Vec::new(),
            aliases: Vec::new(),
            paths: None,
            related: Vec::new(),
            verified: Some("2026-09-01".into()),
            updated: Some("2026-09-20".into()),
            content_hash: String::new(),
            headings: Vec::new(),
            diagnostics: Vec::new(),
            stale: false,
            stale_reasons: Vec::new(),
            space: Some("shop".into()),
            project: Some("shop".into()),
        }
    }

    fn task(id: &str, text: &str) -> L1Task<'static> {
        L1Task { id: Box::leak(id.to_string().into_boxed_str()), epic: None, corpus: text.into() }
    }

    fn chosen_paths(c: &[Chosen]) -> Vec<&str> {
        c.iter().map(|x| x.path.as_str()).collect()
    }

    #[test]
    fn units_count_latin_by_four_and_cyrillic_by_two_rounded_up() {
        assert_eq!(units(""), 0);
        assert_eq!(units("abcd"), 1);
        assert_eq!(units("abcde"), 2);
        assert_eq!(units("абвг"), 2);
        assert_eq!(units("abаб"), 2);
        assert_eq!(units("   "), 1, "whitespace counts as Latin");
        assert_eq!(units("ёж"), 1);
        assert_eq!(units("Ҁҁ"), 1, "the Cyrillic supplement is Cyrillic too");
        assert!(units(&"русский текст".repeat(10)) > units(&"latin text".repeat(10)));
    }

    #[test]
    fn l0_is_metadata_only_with_markers_in_path_order() {
        let mut alpha = page("shop/alpha.md", "Alpha");
        alpha.summary = Some("Alpha summary".into());
        alpha.stale = true;
        let mut draft = page("shop/draft.md", "Draft page");
        draft.status = Some("draft".into());
        let mut unverified = page("shop/unverified.md", "Unverified page");
        unverified.verified = None;
        unverified.doc_type = None;
        let mut old = page("shop/deprecated.md", "Old page");
        old.status = Some("deprecated".into());
        let mut broken = page("shop/broken.md", "Broken page");
        broken.diagnostics = vec!["frontmatter: bad YAML".into()];
        let pages = vec![alpha, broken, old, draft, unverified];
        let l0 = l0_index(&pages).unwrap();
        assert!(!l0.compacted && l0.units <= L0_TARGET_UNITS && l0.pages == 5);
        for m in ["Alpha summary", "[stale]", "[draft]", "[unverified]", "[deprecated]", "[check frontmatter]", "(untyped, current"] {
            assert!(l0.text.contains(m), "{m}: {}", l0.text);
        }
        let order: Vec<&str> = l0
            .text
            .lines()
            .filter_map(|l| l.strip_prefix("- "))
            .map(|l| l.split(" — ").nth(1).unwrap().split(" (").next().unwrap())
            .collect();
        assert_eq!(order, ["Alpha", "Broken page", "Old page", "Draft page", "Unverified page"]);
        assert!(l0.text.contains("- shop/alpha.md — Alpha (guide, current, 2026-09-20): Alpha summary [stale]"), "{}", l0.text);
        assert!(l0_index(&[]).is_none());
    }

    #[test]
    fn a_large_l0_compacts_under_the_target_and_says_what_it_leaves_out() {
        let pages: Vec<DocPage> = (0..30)
            .map(|i| {
                let mut p = page(&format!("shop/ru-{i:02}.md"), &format!("Документ номер {i}"));
                p.summary = Some(format!("Описание страницы номер {i} для проверки компактизации индекса документации"));
                p
            })
            .collect();
        let l0 = l0_index(&pages).unwrap();
        assert!(l0.compacted && l0.units <= L0_TARGET_UNITS && l0.units <= L0_HARD_CAP_UNITS, "{}", l0.units);
        assert!(l0.shown < 30 && l0.shown > 0, "{}", l0.shown);
        assert!(l0.text.contains(&format!("compact: showing {} of 30", l0.shown)));
        assert!(l0.text.contains("more page(s) not listed") && l0.text.contains("draft 0, stale 0"), "{}", l0.text);
    }

    #[test]
    fn l1_ranks_related_over_path_over_words_and_says_why() {
        let mut related = page("shop/zeta.md", "Zeta notes");
        related.related = vec!["g-7".into()];
        let mut by_path = page("shop/export.md", "Export internals");
        by_path.paths = Some(vec!["src/export/**".into()]);
        let mut by_word = page("shop/csv.md", "CSV format");
        by_word.headings = vec!["Delimiters".into()];
        let unrelated = page("shop/other.md", "Billing");
        let pages = vec![by_word, by_path, related, unrelated];
        let c = choose(&pages, &task("G-7", "Export orders as CSV\nChange src/export/csv.rs to add delimiters"), &[]);
        assert_eq!(chosen_paths(&c), ["shop/zeta.md", "shop/export.md", "shop/csv.md"]);
        assert_eq!(c[0].reasons, ["related G-7"]);
        assert!(c[1].reasons.contains(&"path match src/export/csv.rs".to_string()), "{:?}", c[1].reasons);
        assert!(
            c[2].reasons.iter().any(|r| r == "term match CSV") && c[2].reasons.iter().any(|r| r == "term match delimiters"),
            "{:?}",
            c[2].reasons
        );
        assert_eq!((c[0].rank, c[1].rank, c[2].rank), (0, 1, 2));
    }

    #[test]
    fn current_beats_draft_and_deprecated_comes_only_when_related() {
        let mut draft = page("shop/a-draft.md", "Invoices");
        draft.status = Some("draft".into());
        let current = page("shop/b-current.md", "Invoices");
        let mut old = page("shop/c-old.md", "Invoices");
        old.status = Some("deprecated".into());
        // Three of seven pages share the word: rare enough to choose.
        let others = ["Payments", "Refunds", "Catalog", "Shipping"].map(|t| page(&format!("shop/z-{}.md", t.to_lowercase()), t));
        let mut pages = vec![draft, current, old.clone()];
        pages.extend(others);
        let c = choose(&pages, &task("G-1", "Invoices"), &[]);
        assert_eq!(chosen_paths(&c), ["shop/b-current.md", "shop/a-draft.md"], "deprecated stays out");
        let mut old_related = old;
        old_related.related = vec!["G-1".into()];
        let c = choose(&[old_related], &task("G-1", "Invoices"), &[]);
        assert_eq!(c.len(), 1, "a deprecated page related to the task comes");
    }

    #[test]
    fn russian_words_fold_yo_but_not_inflections_and_aliases_bridge() {
        let mut p = page("shop/export.md", "Выгрузка заказов");
        p.aliases = vec!["экспорт".into()];
        let pages = vec![p, page("shop/x.md", "Склад"), page("shop/y.md", "Касса")];
        assert_eq!(chosen_paths(&choose(&pages, &task("G-1", "выгрузка"), &[])), ["shop/export.md"]);
        assert!(choose(&pages, &task("G-1", "выгрузки"), &[]).is_empty(), "no stemming");
        assert_eq!(chosen_paths(&choose(&pages, &task("G-1", "Экспорт в CSV"), &[])), ["shop/export.md"], "aliases bridge");
        let mut yo = page("shop/yo.md", "Ёмкость склада");
        yo.tags = vec!["учёт".into()];
        assert_eq!(chosen_paths(&choose(&[yo], &task("G-1", "емкость и учет"), &[])).len(), 1, "ё and е match");
    }

    #[test]
    fn the_body_never_chooses_a_page() {
        let mut p = page("shop/a.md", "Shipping");
        p.summary = Some("Carriers and rates".into());
        p.headings = vec!["Tracking numbers".into()];
        assert!(term_surface(&p).contains("Tracking numbers") && term_surface(&p).contains("Carriers"));
        let pages = vec![p, page("shop/b.md", "Billing"), page("shop/c.md", "Catalog")];
        assert_eq!(choose(&pages, &task("G-1", "tracking"), &[]).len(), 1);
        assert!(choose(&pages, &task("G-1", "bodyonlyword"), &[]).is_empty());
    }

    #[test]
    fn common_words_and_the_projects_name_choose_nothing() {
        let pages: Vec<DocPage> =
            (0..4).map(|i| page(&format!("shop/p{i}.md"), &format!("Orders page {}", ["alpha", "beta", "gamma", "delta"][i]))).collect();
        assert!(choose(&pages, &task("G-1", "orders"), &[]).is_empty(), "a word on most pages chooses none");
        assert_eq!(choose(&pages, &task("G-1", "gamma"), &[]).len(), 1, "a rare word still chooses");
        let named = vec![page("shop/a.md", "Shop overview"), page("shop/b.md", "Payments"), page("shop/c.md", "Catalog")];
        assert!(choose(&named, &task("G-1", "the shop"), &["shop".into()]).is_empty(), "mentioning the project proves nothing");
        assert!(choose(&named, &task("G-1", "about this and that"), &[]).is_empty(), "function words");
    }

    #[test]
    fn a_long_page_is_clipped_at_sections_with_a_marker_and_within_its_budget() {
        let body: String = (0..30).map(|i| format!("## Section {i}\n\n{}\n\n", "Some text of the section. ".repeat(20))).collect();
        let c = clip_at_sections(&body, L1_PAGE_UNITS);
        assert!(c.clipped && c.shown > 0 && c.shown < c.sections && units(&c.text) <= L1_PAGE_UNITS, "{} units", units(&c.text));
        assert!(c.text.contains(&format!("showing {} of {} sections", c.shown, c.sections)));
        assert!(c.text.trim_end().ends_with("...]"));
        let short = clip_at_sections("# A\n\nshort", 100);
        assert!(!short.clipped && short.text == "# A\n\nshort");
        let one = clip_at_sections(&"word ".repeat(2000), 300);
        assert!(one.clipped && one.shown == 0 && units(&one.text) <= 300, "a single long section is cut by lines");
    }

    #[test]
    fn l1_takes_three_pages_within_the_total_and_marks_stale_and_draft() {
        let mut pages: Vec<DocPage> = (0..5)
            .map(|i| {
                let mut p = page(&format!("shop/p{i}.md"), &format!("Page {i}"));
                p.related = vec![if i < 4 { "G-1" } else { "G-2" }.into()];
                p
            })
            .collect();
        pages[0].stale = true;
        pages[1].status = Some("draft".into());
        pages[2].status = Some("draft".into());
        pages[3].status = Some("draft".into());
        let c = choose(&pages, &task("G-1", "anything"), &[]);
        assert_eq!(c.len(), L1_MAX_PAGES, "four related pages, three chosen");
        assert_eq!(chosen_paths(&c), ["shop/p0.md", "shop/p1.md", "shop/p2.md"], "current first, then by path");
        let body: String = (0..40).map(|i| format!("## Part {i}\n\n{}\n\n", "Long enough text. ".repeat(30))).collect();
        let text = l1_context(&pages, &c, |_| Some(body.clone())).unwrap();
        assert!(units(&text) <= L1_TOTAL_UNITS, "{}", units(&text));
        assert_eq!(text.matches("\n### ").count(), 3, "{text}");
        assert!(
            text.contains("(guide, current, stale) — related G-1") && text.contains("### Page 1 — shop/p1.md (guide, draft) — related G-1"),
            "{text}"
        );
        assert!(text.contains("page clipped at section boundaries"));
        assert!(l1_context(&pages, &[], |_| None).is_none());
    }

    #[test]
    fn required_reading_is_delivered_whole_with_versions_and_missing_pages() {
        let body: String = (0..400).map(|i| format!("## Part {i}\n\n{}\n\n", "Long enough text. ".repeat(10))).collect();
        assert!(units(&body) > L1_TOTAL_UNITS, "the body must exceed the L1 total on purpose");
        let page = RequiredPageView {
            path: "shop/agent-guide.md".into(),
            title: "Agent guide".into(),
            version: "abcdef123456".into(),
            content: body.clone(),
            oversize: false,
        };
        let text = required_section(&[page], &["shop/missing.md".into()]).unwrap();
        assert!(text.starts_with("## Required reading\n\nFollow these pages"), "{text}");
        assert!(text.contains("### Agent guide (`shop/agent-guide.md`, version abcdef123456)"), "{text}");
        assert!(text.contains(body.trim_end()), "the whole body, beyond any L1 page limit");
        assert!(text.contains("- shop/missing.md: missing"), "{text}");
        assert!(!text.contains("clipped") && !text.contains("page clipped"), "required pages are never cut: {text}");
        assert!(required_section(&[], &[]).is_none(), "no required pages means no section");
        let only_missing = required_section(&[], &["shop/gone.md".into()]).unwrap();
        assert!(only_missing.contains("- shop/gone.md: missing") && !only_missing.contains("###"), "{only_missing}");
    }

    #[test]
    fn a_required_change_notice_names_the_page_and_carries_a_bounded_diff() {
        let small = required_change_notice("shop/guide.md", 7, "-old\n+new");
        assert!(small.starts_with("Required page `shop/guide.md` changed (changeset #7)"), "{small}");
        assert!(small.contains("```diff\n-old\n+new\n```") && !small.contains("diff cut"), "{small}");
        let big = required_change_notice("shop/guide.md", 7, &"+line\n".repeat(5000));
        assert!(big.contains("diff cut") && big.chars().count() < 6300, "the diff is bounded");
    }

    #[test]
    fn paths_come_from_the_task_text_without_links() {
        assert_eq!(
            path_candidates("Fix src/export/csv.rs and docs/api.md (see https://example.com/a/b); not a/"),
            ["src/export/csv.rs", "docs/api.md"]
        );
    }
}
