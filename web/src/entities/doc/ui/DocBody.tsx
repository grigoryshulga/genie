// DocBody — Markdown for a docs page. `Markdown.tsx` is owned by another task and
// renders `[[wiki-links]]` literally, so resolved links are rewritten here into a
// sentinel URL that is intercepted on click; unresolved/ambiguous targets stay
// literal (the docs core never guesses a target).

import { type MouseEvent, useMemo } from "react";
import { Markdown } from "@/shared/ui";
import type { DocLink } from "../model.ts";

const SENTINEL = "https://genie.local/docs?p=";

function rewriteWikiLinks(text: string, links: DocLink[]): string {
  if (!text.includes("[[")) return text;
  const byTarget = new Map(links.map((link) => [link.target, link]));
  return text.replace(/\[\[([^\]]+)\]\]/g, (whole, inner: string) => {
    const [rawTarget, rawLabel] = inner.split("|");
    const target = rawTarget.split("#")[0].trim();
    const link = byTarget.get(target);
    if (!link || link.resolution !== "resolved" || !link.targetPath) return whole;
    const label = (rawLabel ?? rawTarget).trim();
    return `[${label}](${SENTINEL}${encodeURIComponent(link.targetPath)})`;
  });
}

export function DocBody({
  text,
  links = [],
  onOpen,
  empty,
}: {
  text: string;
  links?: DocLink[];
  onOpen?: (path: string) => void;
  empty?: string;
}) {
  const prepared = useMemo(() => rewriteWikiLinks(text, links), [text, links]);
  const onClick = (event: MouseEvent<HTMLDivElement>) => {
    const anchor = (event.target as HTMLElement).closest("a");
    const href = anchor?.getAttribute("href") ?? "";
    if (!href.startsWith(SENTINEL)) return;
    event.preventDefault();
    onOpen?.(decodeURIComponent(href.slice(SENTINEL.length)));
  };
  return (
    <div className="doc-body g-prose" onClick={onClick}>
      <Markdown text={prepared} {...(empty !== undefined ? { empty } : {})} />
    </div>
  );
}
