// The project chat's pure logic: what the composer completes, how messages group and merge, and when the
// project manager is answering. No browser APIs and no runtime imports, so the node tests load it directly.

import type { ChatMessage } from "../../shared/api/generated/ChatMessage.ts";
import type { ManagerChat } from "../../shared/api/generated/ManagerChat.ts";

/** Messages per page; a shorter page is the oldest one. */
export const CHAT_PAGE = 50;

/** Consecutive messages from one author within this gap share a header. */
const GROUP_GAP_MS = 5 * 60_000;

/** The project manager's author role on the wire; the web labels it, it never shows the raw role. */
export const MANAGER_ROLE = "orchestrator";
/** The mention the server reads as the project manager (`@pm`, `@manager`, `@менеджер`). */
export const MANAGER_KEY = "pm";
/** The mention target the server stores for the project manager. */
const MANAGER_TARGET = "manager";

/** One entry of the `@` list: what the composer inserts, the line it shows, and the words it matches. */
export interface MentionOption {
  key: string;
  label: string;
  terms: string[];
}

/** The `@` token under the caret: where its `@` sits and what was typed after it. */
export interface MentionAt {
  start: number;
  query: string;
}

/** An `@` that starts a word, followed by word characters up to the caret. */
const MENTION_TAIL = /(^|\s)@([\p{L}\p{N}_.-]*)$/u;

export function mentionAt(text: string, caret: number): MentionAt | null {
  const before = text.slice(0, caret);
  const match = MENTION_TAIL.exec(before);
  if (!match) return null;
  const query = match[2] ?? "";
  return { start: before.length - query.length - 1, query };
}

/** The project manager first, then the people who can be mentioned. A query matches any word that contains it. */
export function mentionOptions(people: { login: string; name?: string }[], query: string): MentionOption[] {
  const q = query.toLowerCase();
  const options: MentionOption[] = [
    { key: MANAGER_KEY, label: "@pm — менеджер проекта", terms: [MANAGER_KEY, "manager", "менеджер"] },
    ...people.map((p) => ({
      key: p.login,
      label: p.name && p.name !== p.login ? `@${p.login} — ${p.name}` : `@${p.login}`,
      terms: [p.login, p.name ?? ""].map((t) => t.toLowerCase()),
    })),
  ];
  return options.filter((o) => !q || o.terms.some((t) => t.includes(q)));
}

/** The text after picking `key`: the `@` token replaced by `@key `, the caret after the space. */
export function insertMention(text: string, caret: number, at: MentionAt, key: string): { text: string; caret: number } {
  const token = `@${key} `;
  return { text: text.slice(0, at.start) + token + text.slice(caret), caret: at.start + token.length };
}

/** The wrap-around step of an arrow key over a list of `count` entries. */
export function nextOption(current: number, count: number, step: 1 | -1): number {
  if (count === 0) return 0;
  return (current + step + count) % count;
}

/** Whether a message was written by a person, not by the project manager. */
export function isManager(message: Pick<ChatMessage, "authorRole">): boolean {
  return message.authorRole === MANAGER_ROLE;
}

/**
 * Whether the project manager is answering: it has chat mail it has not received, or it is working and the
 * newest message that tags it is newer than its newest reply.
 */
export function managerAnswering(messages: readonly ChatMessage[], manager: ManagerChat | undefined): boolean {
  if (!manager) return false;
  if (manager.pending > 0) return true;
  if (manager.state !== "working") return false;
  let tagged = -1;
  let replied = -1;
  for (const m of messages) {
    if (isManager(m)) replied = Math.max(replied, m.id);
    else if (m.mentions.includes(MANAGER_TARGET)) tagged = Math.max(tagged, m.id);
  }
  return tagged > replied;
}

/** One message as the list shows it: `continued` drops its author header (same author, a moment after the last one). */
export interface ChatRow {
  message: ChatMessage;
  continued: boolean;
}

export function chatRows(messages: readonly ChatMessage[]): ChatRow[] {
  return messages.map((message, i) => {
    const prev = messages[i - 1];
    const continued = !!prev && prev.author === message.author && prev.authorRole === message.authorRole && Date.parse(message.at) - Date.parse(prev.at) < GROUP_GAP_MS;
    return { message, continued };
  });
}

/** The messages in id order, each once. Later lists win, so a fresh copy of a message replaces an old one. */
export function mergeMessages(...lists: readonly (readonly ChatMessage[])[]): ChatMessage[] {
  const byId = new Map<number, ChatMessage>();
  for (const list of lists) for (const m of list) byId.set(m.id, m);
  return [...byId.values()].sort((a, b) => a.id - b.id);
}

/** The message a reply points at, when it is loaded. */
export function quotedMessage(messages: readonly ChatMessage[], replyTo: number | null): ChatMessage | undefined {
  return replyTo === null ? undefined : messages.find((m) => m.id === replyTo);
}

/** The first line of a message, cut to `max` characters, with its markup marks dropped: what a quote shows. */
export function excerpt(text: string, max = 140): string {
  const line = text.split("\n").find((l) => l.trim()) ?? "";
  const clean = line.replace(/\*\*|__|`/g, "").replace(/^\s*(#+|[-*+]|\d+\.)\s+/, "").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

/** The name a message is shown under: the project manager's label, a person's display name, else their login. */
export function authorLabel(message: Pick<ChatMessage, "author" | "authorRole">, names: ReadonlyMap<string, string>, managerLabel: string): string {
  if (isManager(message)) return managerLabel;
  return names.get(message.author) ?? message.author;
}

/** Whether to ask for an older page: the newest page was full, or the last older page was. */
export function mayHaveOlder(latestPageFull: boolean, olderPageFull: boolean | undefined): boolean {
  return olderPageFull ?? latestPageFull;
}
