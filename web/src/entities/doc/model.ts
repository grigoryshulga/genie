// UI-side view of the knowledge model. Types are generated from the server's
// Rust structs (`shared/api/types.ts`), so the API contract is checked by the
// compiler on both sides.

import type { Changeset, CommitInfo, DocLink, DocMerge, DocPage, DocRead, DocSearchResult } from "../../shared/api/types.ts";

/** Another author's open changeset that touches a page (`GET /api/docs/page`). */
export interface OpenChangesetRef {
  id: number;
  author: string;
  authorName?: string;
  task?: string | null;
}

/**
 * A page as the server reads it: the page with its text, links and policy, plus the
 * changeset view the server adds. Those fields are optional here while the server
 * side of the vault changesets lands: an older server simply sends none of them.
 */
export type DocReadResult = DocRead & {
  /** The `main` commit this text was read at. */
  version?: string;
  /** The caller's open changeset touching the page, if any (read-your-writes). */
  changeset?: number | null;
  /** Other authors' open changesets touching the page. */
  openChangesets?: OpenChangesetRef[];
  /** Published changesets awaiting review by the section owners. */
  unreviewed?: Changeset[];
};

export type DocType = NonNullable<DocPage["type"]>;
export type DocStatus = NonNullable<DocPage["status"]>;
export type { Changeset, CommitInfo, DocMerge, DocLink, DocPage, DocSearchResult };

/** Follows the order of `DOC_TYPES` in `crates/genie-core/src/vault.rs`. */
export const DOC_TYPES: DocType[] = ["guide", "reference", "decision", "glossary", "runbook", "note"];
/** Follows the order of `DOC_STATUSES` in `crates/genie-core/src/vault.rs`. */
export const DOC_STATUSES: DocStatus[] = ["current", "draft", "deprecated"];

/** Short badge text, as in the mockups (`ref`, `dec`, `note`). */
export const DOC_TYPE_SHORT: Record<DocType, string> = {
  guide: "guide",
  reference: "ref",
  decision: "dec",
  glossary: "gloss",
  runbook: "run",
  note: "note",
};

export const DOC_TYPE_NAME: Record<DocType, string> = {
  guide: "Инструкция",
  reference: "Справочник",
  decision: "Решение",
  glossary: "Глоссарий",
  runbook: "Ранбук",
  note: "Заметка",
};

export const DOC_STATUS_NAME: Record<DocStatus, string> = {
  current: "Актуальна",
  draft: "Черновик",
  deprecated: "Устарела",
};

// ---------------------------------------------------------------- formatting

/** "2 дня назад" from the git-derived `updated` date. */
export function docAgo(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "нет данных";
  const stamp = Date.parse(iso.length === 10 ? `${iso}T00:00:00Z` : iso);
  if (!Number.isFinite(stamp)) return "нет данных";
  const days = Math.floor((now - stamp) / 86_400_000);
  if (days <= 0) return "сегодня";
  if (days === 1) return "вчера";
  if (days < 5) return `${days} дня назад`;
  return `${days} дней назад`;
}

/**
 * Best-effort Russian rendering of the English parser diagnostics (D1: they must
 * be visible). Components keep the raw string in `title`, so nothing is hidden.
 */
export function diagnosticText(raw: string): string {
  const line = /^Line (\d+): (.*)$/.exec(raw);
  if (line) return `Строка ${line[1]}: ${diagnosticText(line[2])}`;
  const simple: [RegExp, (m: RegExpExecArray) => string][] = [
    [/^Duplicate frontmatter field "(.*)"$/, (m) => `Поле «${m[1]}» повторяется`],
    [/^Unknown frontmatter field "(.*)"$/, (m) => `Неизвестное поле frontmatter «${m[1]}»`],
    [/^Field "(.*)" must be a non-empty string$/, (m) => `Поле «${m[1]}» должно быть непустой строкой`],
    [/^Field "(.*)" must be a list of non-empty strings$/, (m) => `Поле «${m[1]}» должно быть списком непустых строк`],
    [/^Field "(.*)" must be one of: (.*)$/, (m) => `Поле «${m[1]}» должно быть одним из: ${m[2]}`],
    [/^Field "(.*)" must be an ISO date in YYYY-MM-DD form$/, (m) => `Поле «${m[1]}» — не дата, нужен формат ГГГГ-ММ-ДД`],
    [/^Field "(.*)" contains invalid repository glob "(.*)"$/, (m) => `Поле «${m[1]}»: неверный glob «${m[2]}»`],
    [/^Field "(.*)": (.*)$/, (m) => `Поле «${m[1]}»: ${m[2]}`],
    [/^expected a frontmatter key and value$/, () => "ожидается поле frontmatter вида «ключ: значение»"],
    [/^expected a YAML string list$/, () => "ожидается список строк"],
    [/^empty item in YAML string list$/, () => "пустой элемент в списке"],
    [/^unclosed quoted list value$/, () => "не закрыта кавычка в списке"],
    [/^unclosed (double|single)-quoted string$/, () => "не закрыта кавычка в строке"],
    [/^unsupported YAML value; use a string or a string list$/, () => "неподдерживаемое значение: нужна строка или список строк"],
    [/^Frontmatter opening delimiter has no closing ---.*$/, () => "у frontmatter нет закрывающего ---, файл прочитан как Markdown"],
  ];
  for (const [re, render] of simple) {
    const m = re.exec(raw);
    if (m) return render(m);
  }
  return raw;
}

/** Best-effort Russian rendering of the English staleness reasons. */
export function staleReasonText(raw: string): string {
  const m = /^(Commit changed|Working-tree change) (.+), matching paths pattern (.+)$/.exec(raw);
  if (!m) return raw;
  return `${m[1] === "Commit changed" ? "Коммит изменил" : "Незакоммиченное изменение"} ${m[2]} — совпало с маской ${m[3]}`;
}

/**
 * Search snippets mark matches with `[` `]`; split them for highlighting. Literal
 * brackets in the page text (`[[wiki-links]]`, `[label](url)`) are masked first so
 * they are never mistaken for a highlight marker (review nit N4).
 */
export function snippetParts(snippet: string): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  const literal = /\[\[[^\]\n]*\]\]|\[[^\]\n]*\]\([^)\n]*\)/g;
  const masked = snippet.replace(literal, (match) => "\u0000".repeat(match.length));
  const re = /\[([^\[\]]*)\]/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(masked))) {
    if (m.index > last) out.push({ text: snippet.slice(last, m.index), hit: false });
    out.push({ text: snippet.slice(m.index + 1, m.index + 1 + m[1].length), hit: true });
    last = m.index + m[0].length;
  }
  if (last < snippet.length) out.push({ text: snippet.slice(last), hit: false });
  return out;
}

/** `docs / reference / documentation.md` breadcrumb segments. */
export function docCrumbs(path: string): string[] {
  return ["docs", ...path.split("/").filter(Boolean)];
}

export function todayIso(now = new Date()): string {
  return now.toISOString().slice(0, 10);
}

/** True for a real `YYYY-MM-DD` calendar date. */
export function isIsoDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const date = new Date(`${value}T00:00:00.000Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}

// ---------------------------------------------------------------- frontmatter

/** The fields the editor form exposes. */
export interface DocFields {
  title: string;
  type: DocType | "";
  status: DocStatus | "";
  verified: string;
  tags: string[];
  paths: string[];
  related: string[];
  /** Not shown in the form, preserved from the loaded page (see the plan, §6.3). */
  summary: string | null;
  aliases: string[];
}

export function fieldsFromPage(page: DocReadResult): DocFields {
  return {
    title: page.title,
    type: page.type ?? "",
    status: page.status ?? "",
    verified: page.verified ?? "",
    tags: page.tags ?? [],
    paths: page.paths ?? [],
    related: page.related ?? [],
    summary: page.summary ?? null,
    aliases: page.aliases ?? [],
  };
}

function yamlScalar(value: string): string {
  if (!value) return '""';
  const needsQuote = /[:#[\]{}&*!|>'"%@`,]/.test(value) || /^\s|\s$/.test(value) || /^(?:[|>&*!{}[\]])/.test(value);
  return needsQuote ? JSON.stringify(value) : value;
}

function yamlList(items: string[]): string {
  return `[${items.map(yamlScalar).join(", ")}]`;
}

/**
 * Frontmatter + body. The parser strips `---\n…---\n`, so appending the body
 * verbatim round-trips `GET /api/docs/page` content back through `POST`.
 */
export function serializeDoc(fields: DocFields, body: string): string {
  const lines = ["---", `title: ${yamlScalar(fields.title.trim())}`];
  if (fields.type) lines.push(`type: ${fields.type}`);
  if (fields.status) lines.push(`status: ${fields.status}`);
  if (fields.summary) lines.push(`summary: ${yamlScalar(fields.summary)}`);
  if (fields.tags.length) lines.push(`tags: ${yamlList(fields.tags)}`);
  if (fields.aliases.length) lines.push(`aliases: ${yamlList(fields.aliases)}`);
  if (fields.paths.length) lines.push(`paths: ${yamlList(fields.paths)}`);
  if (fields.related.length) lines.push(`related: ${yamlList(fields.related)}`);
  if (fields.verified) lines.push(`verified: ${yamlScalar(fields.verified)}`);
  lines.push("---");
  return `${lines.join("\n")}\n${body}`;
}

const TRANSLIT: Record<string, string> = {
  а: "a", б: "b", в: "v", г: "g", д: "d", е: "e", ё: "e", ж: "zh", з: "z", и: "i", й: "y", к: "k", л: "l", м: "m",
  н: "n", о: "o", п: "p", р: "r", с: "s", т: "t", у: "u", ф: "f", х: "h", ц: "c", ч: "ch", ш: "sh", щ: "sch",
  ъ: "", ы: "y", ь: "", э: "e", ю: "yu", я: "ya",
};

/** `SAP dev-800 только read-only` → `sap-dev-800-tolko-read-only`. */
export function slugify(title: string): string {
  const slug = title
    .toLowerCase()
    .split("")
    .map((char) => TRANSLIT[char] ?? char)
    .join("")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60)
    .replace(/-+$/g, "");
  return slug || "page";
}


/** The body as read under the page's title: a first `# heading` that repeats the title is dropped. */
export function bodyUnderTitle(content: string, title: string): string {
  const m = /^\s*#\s+(.+?)\s*#*\s*(?:\r?\n|$)/.exec(content);
  return m && m[1].trim() === title.trim() ? content.slice(m[0].length).replace(/^\s*\n/, "") : content;
}

/** The first paragraph of a body (what a page's summary falls back to), as the server takes it. */
export function firstParagraph(body: string): string | undefined {
  const parts: string[] = [];
  for (const line of body.split(/\r?\n/)) {
    if (!line.trim()) {
      if (parts.length) break;
      continue;
    }
    if (!parts.length && /^\s*(#{1,6}\s|```|~~~)/.test(line)) continue;
    parts.push(line.trim());
  }
  return parts.join(" ").replace(/\s+/g, " ").trim() || undefined;
}
