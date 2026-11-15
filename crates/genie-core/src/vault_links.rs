//! Wiki-link resolution and lossless rewriting use the same project-scoped page inventory.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};

use super::{DocLink, normalize, parse_doc};

pub struct WikiLinks {
    pages: BTreeMap<String, Vec<String>>,
}

impl WikiLinks {
    pub fn new(pages: impl IntoIterator<Item = (String, Vec<String>)>) -> Self {
        Self { pages: pages.into_iter().collect() }
    }

    /// Canonical path first, otherwise a unique filename or alias. Ambiguous targets stay unresolved.
    pub fn resolve(&self, target: &str) -> DocLink {
        let t = target.trim().trim_start_matches("./").trim_end_matches(".md");
        let exact = format!("{t}.md");
        let matches = if self.pages.contains_key(&exact) {
            vec![exact]
        } else if t.contains('.') && self.pages.contains_key(t) {
            vec![t.to_string()]
        } else if t.contains('/') || t.contains('\\') {
            // An explicit path in another root must not fall back to a local basename.
            Vec::new()
        } else {
            let base = normalize(t.rsplit('/').next().unwrap_or(t));
            self.pages
                .iter()
                .filter(|(p, aliases)| {
                    let stem = normalize(Path::new(p).file_stem().and_then(|s| s.to_str()).unwrap_or_default());
                    stem == base || aliases.iter().any(|a| normalize(a) == normalize(t))
                })
                .map(|(p, _)| p.clone())
                .collect::<Vec<_>>()
        };
        let resolution = match matches.len() {
            0 => "unresolved",
            1 => "resolved",
            _ => "ambiguous",
        };
        DocLink {
            target: target.to_string(),
            target_path: (matches.len() == 1).then(|| matches[0].clone()),
            resolution: resolution.to_string(),
            matches,
        }
    }

    /// Replace only targets resolving to the moved page, retaining fragments, labels and all other bytes.
    pub fn rewrite_move(&self, markdown: &str, from: &str, to: &str) -> String {
        let body = parse_doc(markdown, "").body;
        let offset = markdown.len() - body.len();
        let mut text = markdown.to_string();
        for range in target_ranges(&body).into_iter().rev() {
            let target = &body[range.clone()];
            if self.resolve(target).target_path.as_deref() == Some(from) {
                let replacement = if target.ends_with(".md") { to } else { to.strip_suffix(".md").unwrap_or(to) };
                text.replace_range(offset + range.start..offset + range.end, replacement);
            }
        }
        text
    }
}

/// Source ranges come from the Markdown parser, so nested fences, indented blocks,
/// comments, escapes and multiline code spans keep their original bytes.
pub(super) fn target_ranges(body: &str) -> Vec<Range<usize>> {
    Parser::new_ext(body, Options::ENABLE_WIKILINKS)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            if !matches!(
                event,
                Event::Start(
                    Tag::Link { link_type: LinkType::WikiLink { .. }, .. } | Tag::Image { link_type: LinkType::WikiLink { .. }, .. }
                )
            ) {
                return None;
            }
            let source = &body[range.clone()];
            let prefix = if source.starts_with("![[") { 3 } else { 2 };
            let inner = source.get(prefix..source.len().checked_sub(2)?)?;
            let target = inner.split(['|', '#']).next()?.trim();
            if target.is_empty() || inner.contains(['\r', '\n']) {
                return None;
            }
            let leading = inner.len() - inner.trim_start().len();
            let start = range.start + prefix + leading;
            Some(start..start + target.len())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_resolved_targets_without_changing_markdown_examples_or_metadata() {
        let links = WikiLinks::new([("shop/old.md".into(), vec!["старое".into()]), ("shop/other.md".into(), vec![])]);
        let source = concat!(
            "\u{feff}---\r\ntitle: '[[shop/old]]'\r\n---\r\n",
            "# [[old#Heading|Label]]\r\n![[./shop/old.md#^block|текст]] [[старое]] [[ other ]]\r\n",
            "`[[old]]` ``a ` [[old]]`` \\[[old]]\r\n",
            "````md\r\n```\r\n[[old]]\r\n````\r\n~~~md\r\n[[old]]\r\n~~~\r\n",
            "unmatched ` [[old]]\r\n[[ old | label ]] [[#Local]]\r\n",
        );
        let expected = source
            .replace("# [[old#", "# [[shop/new#")
            .replace("![[./shop/old.md", "![[shop/new.md")
            .replace("[[старое]]", "[[shop/new]]")
            .replace("unmatched ` [[old]]", "unmatched ` [[shop/new]]")
            .replace("[[ old |", "[[ shop/new |");
        assert_eq!(links.rewrite_move(source, "shop/old.md", "shop/new.md"), expected);
    }

    #[test]
    fn ambiguous_filenames_and_aliases_do_not_redirect_unrelated_links() {
        let links = WikiLinks::new([("shop/a/old.md".into(), vec!["shared".into()]), ("shop/b/old.md".into(), vec!["shared".into()])]);
        let source = "[[old]] [[shared]] [[shop/a/old]] [[shop/b/old]] [[missing]]";
        assert_eq!(
            links.rewrite_move(source, "shop/a/old.md", "shop/new.md"),
            "[[old]] [[shared]] [[shop/new]] [[shop/b/old]] [[missing]]"
        );
    }

    #[test]
    fn foreign_canonical_paths_never_fall_back_to_a_local_basename() {
        let links = WikiLinks::new([("shop/old.md".into(), vec![])]);
        let source = "[[other/old]] [[other/old.md]] [[missing/old]] [[old]]";
        assert_eq!(links.rewrite_move(source, "shop/old.md", "shop/new.md"), "[[other/old]] [[other/old.md]] [[missing/old]] [[shop/new]]");
        assert_eq!(links.resolve("other/old").resolution, "unresolved");
    }

    #[test]
    fn multiline_code_spans_and_mismatched_backtick_runs_are_preserved() {
        let links = WikiLinks::new([("shop/old.md".into(), vec![])]);
        let source = "``first\n[[old]] ` tail`` [[old]]\n\n`unmatched [[old]]``";
        assert_eq!(
            links.rewrite_move(source, "shop/old.md", "shop/new.md"),
            "``first\n[[old]] ` tail`` [[shop/new]]\n\n`unmatched [[shop/new]]``"
        );
    }

    #[test]
    fn nested_code_and_comments_are_preserved_and_malformed_openers_do_not_hide_links() {
        let links = WikiLinks::new([("shop/old.md".into(), vec![])]);
        let source = concat!(
            "    [[old]]\n\n> ~~~md\n> [[old]]\n> ~~~\n\n",
            "- code:\n\n      [[old]]\n\n<!-- [[old]] -->\n\n",
            "[[unfinished\n[[old]]\n\n- prose\n\n    [[old]]\n",
        );
        let expected = source
            .replace("[[unfinished\n[[old]]", "[[unfinished\n[[shop/new]]")
            .replace("- prose\n\n    [[old]]", "- prose\n\n    [[shop/new]]");
        assert_eq!(links.rewrite_move(source, "shop/old.md", "shop/new.md"), expected);
    }
}
