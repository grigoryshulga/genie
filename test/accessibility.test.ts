// The stylesheets' accessibility contract, apart from the tokens: motion stops when the person asks for
// less of it, colours come from the token layer, and the focus ring is the design system's own.
//
// These are the checks a browser would only show by chance — a component that is not on the audited
// routes still has to keep its promise, so the rules are read from the CSS itself.

import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

const WEB_SRC = path.resolve(import.meta.dirname, "../web/src");

function stylesheets(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) return entry === "__screenshots__" ? [] : stylesheets(full);
    return full.endsWith(".css") ? [full] : [];
  });
}

interface Block {
  file: string;
  selector: string;
  body: string;
  /** Whether the rule sits inside a `prefers-reduced-motion: reduce` query. */
  reduced: boolean;
}

// A small CSS reader: enough of a parser to know each rule's selector, its declarations, and whether a
// reduced-motion query wraps it. Nested at-rules keep their parent's flag.
function parse(file: string, css: string): Block[] {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const blocks: Block[] = [];
  const stack: { selector: string; reduced: boolean; body: string }[] = [];
  let prelude = "";
  for (const char of text) {
    if (char === "}") {
      const done = stack.pop();
      if (done && !done.selector.startsWith("@")) blocks.push({ file, selector: done.selector, body: done.body, reduced: done.reduced });
      prelude = "";
      continue;
    }
    if (char === "{") {
      const selector = prelude.trim().replace(/\s+/g, " ");
      prelude = "";
      const parent = stack[stack.length - 1];
      stack.push({ selector, reduced: Boolean(parent?.reduced) || /prefers-reduced-motion\s*:\s*reduce/.test(selector), body: "" });
      continue;
    }
    // Inside a rule the text is that rule's declarations; inside an at-rule the text is the next
    // rule's selector (an at-rule has no declarations of its own).
    const current = stack[stack.length - 1];
    if (current && !current.selector.startsWith("@")) current.body += char;
    else prelude += char;
  }
  return blocks;
}

const FILES = stylesheets(WEB_SRC);
const BLOCKS = FILES.flatMap((file) => parse(path.relative(WEB_SRC, file), readFileSync(file, "utf8")));
const selectors = (block: Block) => block.selector.split(",").map((s) => s.trim().replace(/\s+/g, " ")).filter(Boolean);
const declarations = (body: string) => body.split(";").map((d) => d.trim()).filter(Boolean);

const startsRule = (block: Block, property: RegExp) => declarations(block.body).some((d) => property.test(d) && !/:\s*none\s*$/.test(d));

// `last()` is the compound a selector ends in (`.g-scrim.g-fsheet-scrim`), and a guard written for one
// class of it (`.g-fsheet-scrim`) counts: the reduced-motion block names the part, not the whole.
const last = (selector: string) => (selector.split(/\s+/).pop() ?? "").replace(/[>+~]/g, "").trim();
const parts = (compound: string) => compound.split(/(?=[.:[#])/).filter(Boolean);
const guarded = (selector: string, guards: Set<string>) => {
  const compound = last(selector);
  const own = parts(compound);
  for (const guard of guards) {
    const wanted = last(guard);
    if (compound === wanted || own.includes(wanted)) return true;
  }
  return false;
};
const stopsRule = (block: Block, property: RegExp) => block.reduced && declarations(block.body).some((d) => property.test(d) && /:\s*none\s*$/.test(d));

const ANIMATION = /^animation(-name)?\s*:/;
const TRANSITION = /^transition(-property)?\s*:/;
const MOTION = /(^|\s)(transform|translate|scale|rotate|top|right|bottom|left|width|height|margin|padding|inset|all)(\s|,|$)/;

test("every animation stops under prefers-reduced-motion", () => {
  const guards = new Set(BLOCKS.filter((b) => stopsRule(b, ANIMATION)).flatMap(selectors));
  const unguarded = BLOCKS.filter((b) => startsRule(b, ANIMATION)).flatMap((b) => selectors(b).filter((s) => !guarded(s, guards)).map((s) => `${b.file} ${s} (${declarations(b.body).find((d) => ANIMATION.test(d))})`));
  assert.deepEqual(unguarded, [], "an animation that keeps running when someone asked for less motion: add it to a prefers-reduced-motion block");
});

test("every transition that moves a shape stops under prefers-reduced-motion", () => {
  const guards = new Set(BLOCKS.filter((b) => stopsRule(b, TRANSITION)).flatMap(selectors));
  const unguarded = BLOCKS.filter((b) => !b.reduced && startsRule(b, TRANSITION) && MOTION.test(declarations(b.body).find((d) => TRANSITION.test(d)) ?? "")).flatMap((b) =>
    selectors(b)
      .filter((s) => !guarded(s, guards))
      .map((s) => `${b.file} ${s} (${declarations(b.body).find((d) => TRANSITION.test(d))})`),
  );
  assert.deepEqual(unguarded, [], "a transition that still moves something under reduced motion: hold it still, or move it into a prefers-reduced-motion block");
});

// Colours belong to the token layer. The legacy palette and its compatibility aliases are gone (H33),
// so the only files allowed a literal are the token layer and the inverted sheet that cannot follow it.
const LITERAL_BUDGET: Record<string, { count: number; why: string }> = {
  "shared/ui/g-tokens.css": { count: 41, why: "the token layer itself: the palette every other rule names" },
  "shared/ui/components.css": { count: 2, why: "the inverted paper sheet's hairlines, which blend with the sheet underneath and so cannot follow the theme" },
};

test("no new hard-coded colours outside the token layer", () => {
  const count = (block: Block) => declarations(block.body).filter((d) => /#[0-9a-fA-F]{3,8}\b|(^|\s)(rgba?|hsla?)\(/.test(d.split(":").slice(1).join(":"))).length;
  const excess = BLOCKS.filter((b) => LITERAL_BUDGET[b.file] === undefined).flatMap((b) => (count(b) > 0 ? [`${b.file} ${b.selector}: ${count(b)} literal colour(s)`] : []));
  assert.deepEqual(excess, [], "a colour literal outside the token layer: use a --g-* token, or record the file's budget with its reason");
  for (const [file, budget] of Object.entries(LITERAL_BUDGET)) {
    const found = BLOCKS.filter((b) => b.file === file).reduce((sum, b) => sum + count(b), 0);
    assert.equal(found, budget.count, `${file} holds ${found} colour literals, its recorded budget is ${budget.count} (${budget.why})`);
  }
});

test("the focus ring is the design system's own and is never a hairline", () => {
  const ring = BLOCKS.filter((b) => !b.reduced && /:focus-visible(?![\w-])/.test(b.selector) && declarations(b.body).some((d) => /^outline\s*:/.test(d)));
  assert.ok(ring.length >= 1, "a :focus-visible rule draws a ring");
  const token = ring.filter((b) => declarations(b.body).some((d) => /^outline\s*:\s*(2px|3px)\s+solid\s+var\(--(g-focus|accent-2)\)/.test(d)));
  assert.ok(token.length >= 1, "the shared focus ring is 2px or more of the focus token");
  for (const block of ring) {
    const outline = declarations(block.body).find((d) => /^outline\s*:/.test(d)) ?? "";
    // The composer owns one ring around its textarea and action bar. Browser flows also verify
    // the computed wrapper ring in both themes; drawing a second textarea ring crosses the bar.
    if (block.file === "widgets/task-detail/ui/detail.css" && block.selector === ".composer .g-textarea:focus, .composer .g-textarea:focus-visible") {
      const wrapper = BLOCKS.find((b) => b.file === block.file && b.selector === ".composer:has(textarea:focus)");
      assert.ok(wrapper && declarations(wrapper.body).some((d) => /^outline\s*:\s*(2px|3px)\s+solid\s+var\(--g-focus\)/.test(d)), "the composer's wrapper must draw the delegated focus ring");
      continue;
    }
    // A field that frames itself on focus draws the one ring on its wrapper; the control inside draws none. Each wrapper must keep that ring.
    if (block.file === "shared/ui/components.css" && block.selector === ".app :is(.doc-chips, .g-search, .docs-filter, .search, .ac-composer, .tv-composer, .composer) :focus-visible") {
      for (const wrapper of [".doc-chips:focus-within", ".g-search:focus-within", ".docs-filter:focus-within", ".search:focus-within", ".ac-composer:focus-within", ".tv-composer:focus-within", ".composer:has(textarea:focus)"]) {
        assert.ok(
          BLOCKS.some((b) => b.selector.split(",").map((s) => s.trim()).includes(wrapper) && declarations(b.body).some((d) => /^outline\s*:\s*(2px|3px)\s+solid\s+var\(--g-focus\)/.test(d))),
          `${wrapper} must draw the focus ring that its control leaves to it`,
        );
      }
      continue;
    }
    assert.ok(!/outline\s*:\s*(none|0)\b/.test(outline), `${block.file} ${block.selector} removes the ring on focus`);
  }
});
