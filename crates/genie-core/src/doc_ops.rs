//! Edit operations on a documentation page: sections, frontmatter fields and whole-page writes.
//!
//! A page is Markdown with an optional `---` frontmatter block. Headings are ATX lines outside
//! fenced code blocks. A section runs from its heading to the next heading of the same or a higher
//! level, so a section's body includes its subsections. Text an operation does not touch is kept
//! as it was; only the blank lines around the edited region are normalised.

use crate::error::{GenieError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::Range;

/// One edit of a documentation page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "op", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum DocOp {
    /// Replace the body of a section (everything after its heading line up to the next heading of the same or higher level). The heading line stays.
    ReplaceSection { heading: String, body: String },
    /// Insert a new section `heading` after the section `after` (after its whole body including subsections); `after: None` appends at the end of the page. `level`: heading level 1-6; default: the level of `after`, or 2.
    InsertSection { after: Option<String>, heading: String, level: Option<u8>, body: String },
    /// Append text at the end of a section's body (before the next heading of the same or higher level), or at the end of the page when `heading` is None.
    Append { heading: Option<String>, text: String },
    /// Remove a section: its heading line and its body, including subsections.
    RemoveSection { heading: String },
    /// Set one frontmatter field. `value`: a JSON string → `field: value`; a JSON array of strings → `field: [a, b]`; JSON null → remove the field. Creates the frontmatter block when the page has none.
    SetMeta {
        field: String,
        #[ts(type = "string | string[] | null")]
        value: serde_json::Value,
    },
    /// Replace the whole page.
    WritePage { content: String },
}

/// Apply operations in order. `page`: the current file text, `None` when the page does not exist yet (then only `WritePage`, `InsertSection` with `after: None`, `Append` with `heading: None` and `SetMeta` are allowed; they start from an empty page).
pub fn apply_ops(page: Option<&str>, ops: &[DocOp]) -> Result<String> {
    let mut doc = page.map_or_else(Page::default, Page::parse);
    let mut exists = page.is_some();
    for op in ops {
        if !exists && !creates_page(op) {
            return Err(GenieError::invalid(
                "the page does not exist; only write-page, insert-section without after, append without heading and set-meta can create it",
            ));
        }
        match op {
            DocOp::ReplaceSection { heading, body } => replace_section(&mut doc, heading, body)?,
            DocOp::InsertSection { after, heading, level, body } => insert_section(&mut doc, after.as_deref(), heading, *level, body)?,
            DocOp::Append { heading, text } => append(&mut doc, heading.as_deref(), text)?,
            DocOp::RemoveSection { heading } => remove_section(&mut doc, heading)?,
            DocOp::SetMeta { field, value } => set_meta(&mut doc, field, value)?,
            DocOp::WritePage { content } => {
                if content.trim().is_empty() {
                    return Err(GenieError::invalid("page content must not be empty"));
                }
                doc = Page::parse(content);
            }
        }
        exists = true;
    }
    let text = doc.render();
    if text.trim().is_empty() {
        return Err(GenieError::invalid("the page would be empty after these operations"));
    }
    Ok(text)
}

fn creates_page(op: &DocOp) -> bool {
    matches!(
        op,
        DocOp::WritePage { .. } | DocOp::InsertSection { after: None, .. } | DocOp::Append { heading: None, .. } | DocOp::SetMeta { .. }
    )
}

/// A page split into its frontmatter lines and body lines (without line terminators).
#[derive(Default)]
struct Page {
    bom: bool,
    frontmatter: Option<Vec<String>>,
    lines: Vec<String>,
}

impl Page {
    fn parse(text: &str) -> Page {
        let bom = text.starts_with('\u{feff}');
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        // Splitting on `\n` keeps any `\r` in the lines, so untouched lines come back unchanged.
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        let close = lines.iter().skip(1).position(|l| is_delimiter(l)).map(|i| i + 1);
        match close {
            Some(close) if lines.first().is_some_and(|l| is_delimiter(l)) => {
                let body = lines.split_off(close + 1);
                lines.truncate(close);
                lines.remove(0);
                Page { bom, frontmatter: Some(lines), lines: body }
            }
            _ => Page { bom, frontmatter: None, lines },
        }
    }

    fn render(&self) -> String {
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        if let Some(frontmatter) = &self.frontmatter {
            out.push_str("---\n");
            for line in frontmatter {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("---\n");
        }
        for line in trim_end_blank(&self.lines) {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

fn is_delimiter(line: &str) -> bool {
    line.trim_end_matches('\r') == "---"
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn text_lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

/// `lines` without trailing blank lines.
fn trim_end_blank(lines: &[String]) -> &[String] {
    let end = lines.iter().rposition(|l| !is_blank(l)).map_or(0, |i| i + 1);
    &lines[..end]
}

/// `lines` without leading and trailing blank lines.
fn trim_blank(lines: &[String]) -> &[String] {
    let trimmed = trim_end_blank(lines);
    let start = trimmed.iter().position(|l| !is_blank(l)).unwrap_or(trimmed.len());
    &trimmed[start..]
}

/// Concatenate the non-empty blocks with one blank line between neighbours. Callers trim the blocks they own.
fn join(blocks: &[&[String]]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for block in blocks.iter().copied().filter(|b| !b.is_empty()) {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.extend_from_slice(block);
    }
    out
}

/// Replace `lines[start..end]` with `body`, keeping one blank line between the body and the neighbouring blocks.
fn set_body(lines: &mut Vec<String>, start: usize, end: usize, body: &[String]) {
    *lines = join(&[&lines[..start], body, &lines[end..]]);
}

struct Heading {
    /// Index of the heading line in the body.
    line: usize,
    level: usize,
    /// Heading texts from the outermost ancestor down to this heading.
    path: Vec<String>,
}

/// Case-folded, whitespace-collapsed form used to match headings.
fn fold(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The line without up to three leading spaces; `None` when it is indented further.
fn unindented(line: &str) -> Option<&str> {
    let text = line.trim_start_matches(' ');
    (line.len() - text.len() <= 3).then_some(text)
}

/// Level and text of an ATX heading line.
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let text = unindented(line.trim_end_matches('\r'))?;
    let level = text.bytes().take_while(|&b| b == b'#').count();
    let rest = &text[level..];
    if !(1..=6).contains(&level) || !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    // A closing run of `#` (`## Title ##`) is not part of the title; `C#` keeps its `#`.
    let title = rest.trim();
    let open = title.trim_end_matches('#');
    let title = if open.is_empty() || open.ends_with(' ') { open } else { title };
    Some((level, title.trim()))
}

/// The fence run at the start of `line`: its character, its length and the text after it.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let text = unindented(line.trim_end_matches('\r'))?;
    let fence = text.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = text.chars().take_while(|&c| c == fence).count();
    (len >= 3).then_some((fence, len, &text[len..]))
}

fn opens_fence(line: &str) -> Option<(char, usize)> {
    let (fence, len, rest) = fence_run(line)?;
    // A backtick info string cannot contain a backtick; otherwise the line is inline code.
    (fence != '`' || !rest.contains('`')).then_some((fence, len))
}

fn closes_fence(line: &str, fence: char, len: usize) -> bool {
    matches!(fence_run(line), Some((c, n, rest)) if c == fence && n >= len && rest.trim().is_empty())
}

/// Every heading of the body, skipping fenced code blocks.
fn headings(lines: &[String]) -> Vec<Heading> {
    let mut found: Vec<Heading> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some((c, n)) = fence {
            if closes_fence(line, c, n) {
                fence = None;
            }
            continue;
        }
        if let Some((c, n)) = opens_fence(line) {
            fence = Some((c, n));
            continue;
        }
        let Some((level, title)) = parse_heading(line) else { continue };
        // The parent is the nearest earlier heading of a higher level; its path already holds its ancestors.
        let mut path = found.iter().rev().find(|h| h.level < level).map(|h| h.path.clone()).unwrap_or_default();
        path.push(title.to_string());
        found.push(Heading { line: index, level, path });
    }
    found
}

/// Whether `wanted` (folded segments) names this heading: the last segment is its own text and the
/// earlier segments appear, in order, among its ancestors.
fn path_matches(path: &[String], wanted: &[String]) -> bool {
    let (Some((last, parents)), Some((own, ancestors))) = (wanted.split_last(), path.split_last()) else {
        return false;
    };
    let mut parents = parents.iter().peekable();
    for ancestor in ancestors {
        if parents.peek().is_some_and(|p| p.as_str() == fold(ancestor)) {
            parents.next();
        }
    }
    parents.peek().is_none() && fold(own) == *last
}

/// The single heading `selector` names: a heading text, or a `Parent > Child` path.
fn find_heading(headings: &[Heading], selector: &str) -> Result<usize> {
    let wanted: Vec<String> = selector.split('>').map(|s| fold(s.trim().trim_start_matches('#'))).collect();
    let matches: Vec<usize> = if wanted.iter().any(String::is_empty) {
        Vec::new()
    } else {
        headings.iter().enumerate().filter(|(_, h)| path_matches(&h.path, &wanted)).map(|(i, _)| i).collect()
    };
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => {
            let listed: Vec<String> = headings.iter().map(|h| h.path.join(" > ")).collect();
            let listed = if listed.is_empty() { "none".to_string() } else { listed.join(", ") };
            Err(GenieError::invalid(format!("heading {selector:?} not found; headings: {listed}")))
        }
        _ => {
            let paths: Vec<String> = matches.iter().map(|&i| headings[i].path.join(" > ")).collect();
            Err(GenieError::invalid(format!("heading {selector:?} is ambiguous; use a path: {}", paths.join(", "))))
        }
    }
}

/// Index of the first line after the section of `headings[index]`: the next heading of the same or a higher level.
fn section_end(headings: &[Heading], index: usize, len: usize) -> usize {
    let level = headings[index].level;
    headings[index + 1..].iter().find(|h| h.level <= level).map_or(len, |h| h.line)
}

fn replace_section(page: &mut Page, heading: &str, body: &str) -> Result<()> {
    let headings = headings(&page.lines);
    let index = find_heading(&headings, heading)?;
    let end = section_end(&headings, index, page.lines.len());
    let body = text_lines(body);
    set_body(&mut page.lines, headings[index].line + 1, end, trim_blank(&body));
    Ok(())
}

fn insert_section(page: &mut Page, after: Option<&str>, heading: &str, level: Option<u8>, body: &str) -> Result<()> {
    let headings = headings(&page.lines);
    let (at, default_level) = match after {
        Some(after) => {
            let index = find_heading(&headings, after)?;
            (section_end(&headings, index, page.lines.len()), headings[index].level)
        }
        None => (page.lines.len(), 2),
    };
    let level = match level {
        Some(l @ 1..=6) => usize::from(l),
        Some(l) => return Err(GenieError::invalid(format!("heading level must be 1-6, got {l}"))),
        None => default_level,
    };
    let title = heading.trim().trim_start_matches('#').trim();
    if title.is_empty() || title.contains(['\r', '\n']) {
        return Err(GenieError::invalid("a new section needs a heading text on one line"));
    }
    let heading_line = vec![format!("{} {title}", "#".repeat(level))];
    let body = text_lines(body);
    let section = join(&[heading_line.as_slice(), trim_blank(&body)]);
    page.lines = join(&[trim_end_blank(&page.lines[..at]), &section, &page.lines[at..]]);
    Ok(())
}

fn append(page: &mut Page, heading: Option<&str>, text: &str) -> Result<()> {
    let text = text_lines(text);
    let text = trim_blank(&text);
    match heading {
        None => page.lines = join(&[trim_end_blank(&page.lines), text]),
        Some(heading) => {
            let headings = headings(&page.lines);
            let index = find_heading(&headings, heading)?;
            let start = headings[index].line + 1;
            let end = section_end(&headings, index, page.lines.len());
            let body = join(&[trim_blank(&page.lines[start..end]), text]);
            set_body(&mut page.lines, start, end, &body);
        }
    }
    Ok(())
}

fn remove_section(page: &mut Page, heading: &str) -> Result<()> {
    let headings = headings(&page.lines);
    let index = find_heading(&headings, heading)?;
    let start = headings[index].line;
    let end = section_end(&headings, index, page.lines.len());
    page.lines = join(&[trim_end_blank(&page.lines[..start]), &page.lines[end..]]);
    Ok(())
}

fn valid_field(field: &str) -> bool {
    let mut chars = field.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A YAML value as written in the frontmatter: JSON-quoted when a plain scalar would not read back unchanged.
fn yaml_scalar(text: &str, in_list: bool) -> Result<String> {
    let quoted = text.is_empty()
        || text.trim() != text
        || text.contains([':', '#', '\n', '\r'])
        || text.starts_with(['"', '\'', '[', ']', '{', '}'])
        || (in_list && text.contains(','));
    Ok(if quoted { serde_json::to_string(text)? } else { text.to_string() })
}

/// Line ranges of the top-level `field` entries: the key line and the nested or `- ` lines after it.
fn field_groups(frontmatter: &[String], field: &str) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < frontmatter.len() {
        if key_of(&frontmatter[i]) == Some(field) {
            let mut end = i + 1;
            while end < frontmatter.len() && is_nested(&frontmatter[end]) {
                end += 1;
            }
            groups.push(i..end);
            i = end;
        } else {
            i += 1;
        }
    }
    groups
}

fn key_of(line: &str) -> Option<&str> {
    if line.is_empty() || line.starts_with([' ', '\t', '#']) {
        return None;
    }
    line.split_once(':').map(|(key, _)| key.trim())
}

fn is_nested(line: &str) -> bool {
    line.starts_with([' ', '\t']) || line == "-" || line.starts_with("- ")
}

fn set_meta(page: &mut Page, field: &str, value: &Value) -> Result<()> {
    if !valid_field(field) {
        return Err(GenieError::invalid(format!("frontmatter field {field:?} must match ^[A-Za-z][A-Za-z0-9_-]*$")));
    }
    let text = match value {
        Value::Null => None,
        Value::String(s) => Some(yaml_scalar(s, false)?),
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|item| match item {
                    Value::String(s) => yaml_scalar(s, true),
                    _ => Err(GenieError::invalid(format!("frontmatter field {field:?} lists must contain only strings"))),
                })
                .collect::<Result<Vec<_>>>()?;
            Some(format!("[{}]", items.join(", ")))
        }
        _ => return Err(GenieError::invalid(format!("frontmatter field {field:?} takes a string, a list of strings or null"))),
    };
    let line = text.map(|text| format!("{field}: {text}"));
    if page.frontmatter.is_none() {
        if line.is_none() {
            return Ok(());
        }
        if page.lines.first().is_some_and(|l| is_delimiter(l)) {
            return Err(GenieError::invalid("frontmatter opening delimiter has no closing ---"));
        }
    }
    let fields = page.frontmatter.take().unwrap_or_default();
    let groups = field_groups(&fields, field);
    // A replaced field stays where its first occurrence was; a new field goes after the last entry.
    let at = match groups.first() {
        Some(group) => group.start,
        None => trim_end_blank(&fields).len(),
    };
    let mut kept: Vec<String> =
        fields.into_iter().enumerate().filter(|(i, _)| !groups.iter().any(|g| g.contains(i))).map(|(_, l)| l).collect();
    if let Some(line) = line {
        kept.insert(at, line);
    }
    page.frontmatter = Some(kept);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(page: &str, ops: Vec<DocOp>) -> Result<String> {
        apply_ops(Some(page), &ops)
    }

    fn replace(heading: &str, body: &str) -> DocOp {
        DocOp::ReplaceSection { heading: heading.into(), body: body.into() }
    }

    fn meta(field: &str, value: Value) -> DocOp {
        DocOp::SetMeta { field: field.into(), value }
    }

    #[test]
    fn replaces_a_nested_section_by_path() {
        let page = "# Guide\n\n## Setup\n\nold\n\n### Install\n\nold install\n\n## Usage\n\nuse\n";
        let out = run(page, vec![replace("Guide > Install", "new install\n\n\n")]).unwrap();
        assert_eq!(out, "# Guide\n\n## Setup\n\nold\n\n### Install\n\nnew install\n\n## Usage\n\nuse\n");
    }

    #[test]
    fn ambiguous_heading_lists_the_paths() {
        let page = "# A\n\n## Notes\n\none\n\n# B\n\n## Notes\n\ntwo\n";
        let err = run(page, vec![replace("Notes", "x")]).unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("A > Notes") && err.contains("B > Notes"), "{err}");
        let out = run(page, vec![replace("  b  >  NOTES ", "three")]).unwrap();
        assert_eq!(out, "# A\n\n## Notes\n\none\n\n# B\n\n## Notes\n\nthree\n");
    }

    #[test]
    fn missing_heading_lists_the_page_headings() {
        let page = "# A\n\n## Notes\n";
        let err = run(page, vec![replace("Nope", "x")]).unwrap_err().to_string();
        assert_eq!(err, "heading \"Nope\" not found; headings: A, A > Notes");
    }

    #[test]
    fn headings_in_code_fences_are_ignored() {
        let page = "# Doc\n\n```md\n# Not a heading\n```\n\n~~~\n## Also not\n~~~\n\n## Real\n\nbody\n";
        assert!(run(page, vec![replace("Not a heading", "x")]).is_err());
        assert!(run(page, vec![replace("Also not", "x")]).is_err());
        let out = run(page, vec![replace("## Real", "new")]).unwrap();
        assert_eq!(out, "# Doc\n\n```md\n# Not a heading\n```\n\n~~~\n## Also not\n~~~\n\n## Real\n\nnew\n");
    }

    #[test]
    fn insert_after_a_section_lands_after_its_subsections() {
        let page = "# A\n\nintro\n\n## A1\n\nx\n\n## B\n\nb\n";
        let op = DocOp::InsertSection { after: Some("A".into()), heading: "New".into(), level: None, body: "n\n".into() };
        let out = run(page, vec![op]).unwrap();
        assert_eq!(out, "# A\n\nintro\n\n## A1\n\nx\n\n## B\n\nb\n\n# New\n\nn\n");
    }

    #[test]
    fn insert_with_level_after_a_subsection() {
        let page = "# A\n\n## A1\n\na1\n\n## B\n";
        let op = DocOp::InsertSection { after: Some("A1".into()), heading: "Next".into(), level: Some(3), body: String::new() };
        let out = run(page, vec![op]).unwrap();
        assert_eq!(out, "# A\n\n## A1\n\na1\n\n### Next\n\n## B\n");
    }

    #[test]
    fn insert_at_end_defaults_to_level_two() {
        let page = "# A\n\nx\n";
        let op = DocOp::InsertSection { after: None, heading: "Z".into(), level: None, body: "z\n\n\n".into() };
        assert_eq!(run(page, vec![op]).unwrap(), "# A\n\nx\n\n## Z\n\nz\n");
    }

    #[test]
    fn appends_to_a_section_and_to_the_page() {
        let page = "# A\n\n## A1\n\na1\n\n# B\n\nb\n";
        let ops =
            vec![DocOp::Append { heading: Some("A".into()), text: "more\n".into() }, DocOp::Append { heading: None, text: "end".into() }];
        assert_eq!(run(page, ops).unwrap(), "# A\n\n## A1\n\na1\n\nmore\n\n# B\n\nb\n\nend\n");
    }

    #[test]
    fn removes_a_section_with_its_subsections() {
        let page = "# A\n\nx\n\n## A1\n\na1\n\n# B\n\nb\n";
        let out = run(page, vec![DocOp::RemoveSection { heading: "A".into() }]).unwrap();
        assert_eq!(out, "# B\n\nb\n");

        let page = "# A\n\nx\n\n## A1\n\na1\n\n## A2\n\na2\n";
        let out = run(page, vec![DocOp::RemoveSection { heading: "A1".into() }]).unwrap();
        assert_eq!(out, "# A\n\nx\n\n## A2\n\na2\n");
    }

    #[test]
    fn set_meta_replaces_adds_lists_and_removes_fields() {
        let page = "---\ntitle: Old\nstatus: draft\n---\n# Body\n\ntext\n";
        let ops = vec![
            meta("title", json!("New: one")),
            meta("summary", json!("short")),
            meta("tags", json!(["a", "b: c"])),
            meta("status", Value::Null),
        ];
        let out = run(page, ops).unwrap();
        assert_eq!(out, "---\ntitle: \"New: one\"\nsummary: short\ntags: [a, \"b: c\"]\n---\n# Body\n\ntext\n");
    }

    #[test]
    fn set_meta_creates_frontmatter_when_missing() {
        assert_eq!(run("# Hi\n", vec![meta("title", json!("T"))]).unwrap(), "---\ntitle: T\n---\n# Hi\n");
        assert_eq!(run("# Hi\n", vec![meta("title", Value::Null)]).unwrap(), "# Hi\n");
    }

    #[test]
    fn set_meta_replaces_a_block_style_list() {
        let page = "---\ntags:\n  - one\n  - two\ntitle: X\n---\n# H\n";
        assert_eq!(run(page, vec![meta("tags", json!(["z"]))]).unwrap(), "---\ntags: [z]\ntitle: X\n---\n# H\n");
        let page = "---\ntags:\n- one\n- two\ntitle: X\n---\n# H\n";
        assert_eq!(run(page, vec![meta("tags", json!(["z"]))]).unwrap(), "---\ntags: [z]\ntitle: X\n---\n# H\n");
    }

    #[test]
    fn set_meta_keeps_unknown_fields_in_order() {
        let page = "---\nlegacy_id: 42\nowner:\n  name: me\nstatus: draft\n---\n# H\n";
        let out = run(page, vec![meta("status", json!("published"))]).unwrap();
        assert_eq!(out, "---\nlegacy_id: 42\nowner:\n  name: me\nstatus: published\n---\n# H\n");
    }

    #[test]
    fn set_meta_rejects_bad_fields_and_value_types() {
        assert!(run("# H\n", vec![meta("bad field", json!("x"))]).is_err());
        assert!(run("# H\n", vec![meta("1st", json!("x"))]).is_err());
        assert!(run("# H\n", vec![meta("count", json!(3))]).is_err());
    }

    #[test]
    fn matches_russian_headings_case_insensitively() {
        let page = "# Документация\n\n## Установка\n\nстарый текст\n\n## Прочее\n\nx\n";
        let out = run(page, vec![replace("  УСТАНОВКА  ", "новый текст")]).unwrap();
        assert_eq!(out, "# Документация\n\n## Установка\n\nновый текст\n\n## Прочее\n\nx\n");
        let out = run(page, vec![replace("документация > установка", "в пути")]).unwrap();
        assert_eq!(out, "# Документация\n\n## Установка\n\nв пути\n\n## Прочее\n\nx\n");
    }

    #[test]
    fn missing_page_starts_from_empty() {
        assert_eq!(apply_ops(None, &[DocOp::Append { heading: None, text: "hello".into() }]).unwrap(), "hello\n");
        let ops = vec![
            DocOp::WritePage { content: "# Created\n".into() },
            DocOp::Append { heading: Some("Created".into()), text: "more".into() },
        ];
        assert_eq!(apply_ops(None, &ops).unwrap(), "# Created\n\nmore\n");
        let ops =
            vec![meta("title", json!("x")), DocOp::InsertSection { after: None, heading: "Head".into(), level: None, body: "b".into() }];
        assert_eq!(apply_ops(None, &ops).unwrap(), "---\ntitle: x\n---\n## Head\n\nb\n");
    }

    #[test]
    fn missing_page_rejects_ops_that_need_a_heading() {
        assert!(apply_ops(None, &[replace("A", "x")]).is_err());
        assert!(apply_ops(None, &[DocOp::RemoveSection { heading: "A".into() }]).is_err());
    }

    #[test]
    fn empty_results_are_rejected() {
        assert!(apply_ops(Some(""), &[]).is_err());
        assert!(apply_ops(None, &[DocOp::WritePage { content: "  \n".into() }]).is_err());
        assert!(run("# A\n", vec![DocOp::RemoveSection { heading: "A".into() }]).is_err());
    }
}
