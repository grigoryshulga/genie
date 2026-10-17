import { Mark } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import { TableKit } from "@tiptap/extension-table";
import TaskList from "@tiptap/extension-task-list";
import TaskItem from "@tiptap/extension-task-item";
import { MarkdownManager } from "@tiptap/markdown";

/** Keep vault links as their original syntax through visual edits, including aliases and headings. */
const WikiLink = Mark.create({
  name: "wikiLink",
  priority: 50,
  inclusive: false,
  addAttributes: () => ({ target: { default: "" }, alias: { default: false }, embed: { default: false } }),
  parseHTML: () => [{ tag: "span[data-wiki-link]", getAttrs: (element) => ({ target: element.getAttribute("data-wiki-link"), alias: element.hasAttribute("data-wiki-alias"), embed: element.hasAttribute("data-wiki-embed") }) }],
  renderHTML: ({ mark }) => ["span", { "data-wiki-link": mark.attrs.target, ...(mark.attrs.alias ? { "data-wiki-alias": "" } : {}), ...(mark.attrs.embed ? { "data-wiki-embed": "" } : {}), class: "doc-wiki-link" }, 0],
  markdownTokenizer: {
    name: "wikiLink",
    level: "inline",
    start: (source) => source.search(/!?\[\[/),
    tokenize: (source) => {
      const match = /^!?\[\[([^\]\n]+)\]\]/.exec(source);
      return match ? { type: "wikiLink", raw: match[0], text: match[1].split("|").at(-1) } : undefined;
    },
  },
  parseMarkdown: (token, helpers) => {
    const raw = String(token.raw ?? "");
    const inner = raw.replace(/^!?\[\[/, "").replace(/\]\]$/, "");
    return helpers.applyMark("wikiLink", [{ type: "text", text: String(token.text ?? inner) }], { target: inner.split("|")[0], alias: inner.includes("|"), embed: raw.startsWith("!") });
  },
  renderMarkdown: (node, helpers) => `${node.attrs?.embed ? "!" : ""}[[${node.attrs?.alias ? `${node.attrs.target}|` : ""}${helpers.renderChildren(node)}]]`,
});

export function articleExtensions() {
  return [
    StarterKit.configure({ underline: false, link: { openOnClick: false } }),
    Image.configure({ inline: true }), TableKit, TaskList, TaskItem, WikiLink,
  ];
}

/** The same schema is used in the editor and round-trip checks. No DOM is required. */
export const articleMarkdown = new MarkdownManager({ extensions: articleExtensions(), markedOptions: { gfm: true } });

export function visualEditingIssue(markdown: string): string | undefined {
  let hasHtml = false;
  articleMarkdown.instance.walkTokens(articleMarkdown.instance.lexer(markdown), (token) => {
    if (token.type === "html") hasHtml = true;
  });
  // The visual schema cannot represent arbitrary HTML. Keep that actual source editable intact.
  return hasHtml ? "Статья содержит HTML. Используйте Markdown для редактирования этих блоков." : undefined;
}
