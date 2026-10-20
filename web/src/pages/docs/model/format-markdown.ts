export type ArticleFormat = "bold" | "italic" | "strike" | "code" | "h1" | "h2" | "h3" | "bullet" | "ordered" | "quote" | "codeBlock" | "divider" | "link";
export interface MarkdownEdit { text: string; start: number; end: number }

/** Apply toolbar commands to a selection without replacing unrelated lines. */
export function formatMarkdown(text: string, start: number, end: number, command: ArticleFormat, url = "https://"): MarkdownEdit {
  const selected = text.slice(start, end);
  const replace = (from: number, to: number, value: string, selectionStart = 0, selectionEnd = value.length): MarkdownEdit => ({
    text: text.slice(0, from) + value + text.slice(to), start: from + selectionStart, end: from + selectionEnd,
  });
  const wrappers = { bold: "**", italic: "*", strike: "~~", code: "`" };
  if (command in wrappers) {
    const marker = wrappers[command as keyof typeof wrappers];
    if (text.slice(start - marker.length, start) === marker && text.slice(end, end + marker.length) === marker) {
      return replace(start - marker.length, end + marker.length, selected);
    }
    const content = selected || "текст";
    return replace(start, end, `${marker}${content}${marker}`, marker.length, marker.length + content.length);
  }
  if (command === "link") {
    const label = selected || "ссылка";
    const safeUrl = url.replace(/\\/g, "\\\\").replace(/\(/g, "\\(").replace(/\)/g, "\\)");
    return replace(start, end, `[${label}](${safeUrl})`, 1, label.length + 1);
  }
  if (command === "divider") {
    const before = start && !text.slice(0, start).endsWith("\n\n") ? "\n\n" : "";
    const block = `${before}---\n\n`;
    return replace(start, end, block, block.length, block.length);
  }
  const from = start === 0 ? 0 : text.lastIndexOf("\n", start - 1) + 1;
  const nextLine = text.indexOf("\n", end > start && text[end - 1] === "\n" ? end - 1 : end);
  const to = nextLine === -1 ? text.length : nextLine;
  const lines = text.slice(from, to).split("\n");
  if (command === "codeBlock") return replace(from, to, `\`\`\`\n${lines.join("\n")}\n\`\`\``, 4, 4 + to - from);
  const prefix = command === "h1" ? "# " : command === "h2" ? "## " : command === "h3" ? "### " : command === "quote" ? "> " : "- ";
  const strip = /^(?:#{1,6}\s+|>\s?|[-*+]\s+|\d+\.\s+)/;
  const formatted = lines.map((line, i) => `${command === "ordered" ? `${i + 1}. ` : prefix}${line.replace(strip, "")}`).join("\n");
  return replace(from, to, formatted);
}
