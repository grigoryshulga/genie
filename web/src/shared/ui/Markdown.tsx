// Minimal, safe markdown → React (no HTML injection): headings, paragraphs,
// lists, task lists, quotes, fenced code, inline code, bold, italics, links.

import { Fragment, type ReactNode } from "react";

function inline(text: string, key = 0): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*\s][^*]*\*)|(\[[^\]]+\]\((https?:\/\/[^)\s]+)\))/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const k = `${key}-${i++}`;
    if (m[1]) out.push(<code key={k}>{m[1].slice(1, -1)}</code>);
    else if (m[2]) out.push(<b key={k}>{m[2].slice(2, -2)}</b>);
    else if (m[3]) out.push(<i key={k}>{m[3].slice(1, -1)}</i>);
    else if (m[4]) {
      const label = m[4].slice(1, m[4].indexOf("]("));
      out.push(
        <a key={k} href={m[5]} target="_blank" rel="noreferrer noopener">
          {label}
        </a>,
      );
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

type Align = "left" | "center" | "right" | null;

// Split a GFM table row into trimmed cells, honouring escaped pipes (`\|`)
// and pipes inside code spans, and dropping one optional leading/trailing pipe.
function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (/\|$/.test(s) && !/\\\|$/.test(s)) s = s.slice(0, -1);
  const cells: string[] = [];
  let cur = "";
  let inCode = false;
  for (let j = 0; j < s.length; j++) {
    const ch = s[j];
    if (ch === "\\" && s[j + 1] === "|") {
      cur += "|";
      j++;
      continue;
    }
    if (ch === "`") inCode = !inCode;
    if (ch === "|" && !inCode) {
      cells.push(cur.trim());
      cur = "";
      continue;
    }
    cur += ch;
  }
  cells.push(cur.trim());
  return cells;
}

function hasCellDivider(line: string): boolean {
  let inCode = false;
  for (let j = 0; j < line.length; j++) {
    if (line[j] === "`") inCode = !inCode;
    else if (line[j] === "|" && !inCode && line[j - 1] !== "\\") return true;
  }
  return false;
}

// The delimiter row (`--- | :--: | ---:`) that turns a row into a table header.
function parseDelimiter(line: string): Align[] | null {
  const cells = splitRow(line);
  if (!cells.length) return null;
  const align: Align[] = [];
  for (const cell of cells) {
    if (!/^:?-+:?$/.test(cell)) return null;
    align.push(cell.startsWith(":") && cell.endsWith(":") ? "center" : cell.endsWith(":") ? "right" : cell.startsWith(":") ? "left" : null);
  }
  return align;
}

function tableStart(lines: string[], i: number): { header: string[]; align: Align[] } | null {
  if (i + 1 >= lines.length || !hasCellDivider(lines[i])) return null;
  const align = parseDelimiter(lines[i + 1]);
  if (!align) return null;
  const header = splitRow(lines[i]);
  if (header.length !== align.length) return null;
  return { header, align };
}

export function Markdown({ text, empty = "Пусто" }: { text: string; empty?: string }) {
  if (!text.trim()) return <div className="md"><span className="empty-md">{empty}</span></div>;
  const lines = text.replace(/\r/g, "").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let k = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    if (line.startsWith("```")) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) code.push(lines[i++]);
      i++;
      blocks.push(
        <pre key={k++}>
          <code>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) {
      const Tag = `h${Math.min(4, h[1].length + 1)}` as "h2";
      blocks.push(<Tag key={k++}>{inline(h[2], k)}</Tag>);
      i++;
      continue;
    }
    if (/^\s*([-*]|\d+\.)\s+/.test(line)) {
      const ordered = /^\s*\d+\./.test(line);
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*([-*]|\d+\.)\s+/.test(lines[i])) {
        const body = lines[i].replace(/^\s*([-*]|\d+\.)\s+/, "");
        const task = /^\[( |x)\]\s+(.*)$/.exec(body);
        items.push(
          <li key={i}>
            {task ? (
              <>
                <input type="checkbox" checked={task[1] === "x"} readOnly aria-label={task[2]} /> {inline(task[2], i)}
              </>
            ) : (
              inline(body, i)
            )}
          </li>,
        );
        i++;
      }
      blocks.push(ordered ? <ol key={k++}>{items}</ol> : <ul key={k++}>{items}</ul>);
      continue;
    }
    if (line.startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) quote.push(lines[i++].replace(/^>\s?/, ""));
      blocks.push(<blockquote key={k++}>{inline(quote.join(" "), k)}</blockquote>);
      continue;
    }
    const table = tableStart(lines, i);
    if (table) {
      const cols = table.header.length;
      const cellAlign: (Align | undefined)[] = table.align;
      const head = table.header.map((cell, ci) => (
        <th key={ci} style={cellAlign[ci] ? { textAlign: cellAlign[ci] } : undefined}>
          {inline(cell, ci)}
        </th>
      ));
      i += 2;
      const rows: ReactNode[] = [];
      while (i < lines.length && lines[i].trim() && hasCellDivider(lines[i])) {
        const cells = splitRow(lines[i]);
        while (cells.length < cols) cells.push("");
        rows.push(
          <tr key={i}>
            {cells.slice(0, cols).map((cell, ci) => (
              <td key={ci} style={cellAlign[ci] ? { textAlign: cellAlign[ci] } : undefined}>
                {inline(cell, ci)}
              </td>
            ))}
          </tr>,
        );
        i++;
      }
      blocks.push(
        <div className="md-table-wrap" key={k++}>
          <table className="md-table">
            <thead>
              <tr>{head}</tr>
            </thead>
            <tbody>{rows}</tbody>
          </table>
        </div>,
      );
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(```|#{1,4}\s|>|\s*([-*]|\d+\.)\s)/.test(lines[i]) && !tableStart(lines, i)) para.push(lines[i++]);
    blocks.push(
      <p key={k++}>
        {para.map((p, j) => (
          <Fragment key={j}>
            {j > 0 && <br />}
            {inline(p, j)}
          </Fragment>
        ))}
      </p>,
    );
  }
  return <div className="md">{blocks}</div>;
}
