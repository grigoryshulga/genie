import { test } from "node:test";
import assert from "node:assert/strict";
import { articleMarkdown, visualEditingIssue } from "../web/src/pages/docs/model/rich-markdown.ts";
import { formatMarkdown } from "../web/src/pages/docs/model/format-markdown.ts";

test("visual editing round-trips article formatting and vault links without losing their meaning", () => {
  for (const source of [
    "# Title\n\n## Topic\n\n**Bold**, *italic*, ~~strike~~ and `code`.\n\n> Quote\n\n---",
    "**Bold [[guide#Session|Read more]] text** and *[[another]]*",
    "[[guide#Session|Read more]] and ![[architecture/diagram]]\n\n`[[not-a-link]]`",
    "1. First\n2. Second\n\n- Bullet\n  - Nested\n\n- [x] Done\n- [ ] To do",
    '| Name | Value |\n| --- | --- |\n| A | B |\n\n![Image](images/test.png "Example")',
    '```typescript\nconst bracket = "[[literal]]";\n```\n\n[Web](https://example.com)\n\nHard  \nbreak',
    "[Reference][doc]\n\n[doc]: https://example.com/docs",
  ]) {
    assert.equal(visualEditingIssue(source), undefined);
    const document = articleMarkdown.parse(source);
    const saved = articleMarkdown.serialize(document);
    assert.deepEqual(articleMarkdown.parse(saved), document, saved);
    if (source.includes("[[guide#Session|Read more]]")) {
      assert.ok(saved.includes("[[guide#Session|Read more]]"));

    }
    if (source.includes("![[architecture/diagram]]")) assert.ok(saved.includes("![[architecture/diagram]]"));
  }
});

test("actual HTML requires source editing while code containing HTML remains visually editable", () => {
  assert.match(visualEditingIssue('<details>\n<summary>Details</summary>\nText\n</details>') ?? "", /HTML/);
  assert.match(visualEditingIssue("Text <span>HTML</span>") ?? "", /HTML/);
  assert.equal(visualEditingIssue('Code `<span>HTML</span>`\n\n```html\n<div>example</div>\n```'), undefined);
});

test("source formatting preserves surrounding text, selects content and can remove marks", () => {
  const bold = formatMarkdown("One two three", 4, 7, "bold");
  assert.deepEqual(bold, { text: "One **two** three", start: 6, end: 9 });
  assert.deepEqual(formatMarkdown(bold.text, bold.start, bold.end, "bold"), { text: "One two three", start: 4, end: 7 });
  const italic = formatMarkdown("", 0, 0, "italic");
  assert.equal(italic.text.slice(italic.start, italic.end), "текст");
  assert.equal(formatMarkdown("Link", 0, 4, "link", "https://example.com/a(b)").text, "[Link](https://example.com/a\\(b\\))");
});

test("source block commands apply to whole selected lines without duplicating existing prefixes", () => {
  assert.equal(formatMarkdown("First\nSecond\nThird", 2, 10, "ordered").text, "1. First\n2. Second\nThird");
  assert.equal(formatMarkdown("# Heading", 3, 3, "h2").text, "## Heading");
  assert.equal(formatMarkdown("First\nSecond", 0, 6, "bullet").text, "- First\nSecond");
  assert.equal(formatMarkdown("Example", 0, 7, "codeBlock").text, "```\nExample\n```");
  assert.equal(formatMarkdown("Before", 6, 6, "divider").text, "Before\n\n---\n\n");
});
