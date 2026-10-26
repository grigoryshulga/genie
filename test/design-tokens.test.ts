// The design tokens' accessibility contract: every text, control, and focus pairing the design system
// documents must hold its WCAG contrast ratio in both themes, so a palette edit cannot silently break it.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { PROGRESS } from "../web/src/entities/task/model.ts";

const CSS = readFileSync(path.resolve(import.meta.dirname, "../web/src/shared/ui/g-tokens.css"), "utf8");

// Declarations of the first rule whose selector matches `selector` exactly.
const declarations = (selector: string): Map<string, string> => {
  const match = CSS.replace(/\/\*[\s\S]*?\*\//g, "").match(new RegExp(`(?:^|\\})\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `rule ${selector} exists in g-tokens.css`);
  return new Map([...match[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()]));
};

// A theme is the base rule overlaid with its own overrides; `var()` references resolve against the merged set.
const theme = (selector: string) => {
  const merged = new Map([...declarations(":root"), ...(selector === ":root" ? [] : declarations(selector))]);
  const resolve = (name: string, depth = 0): string => {
    const value = merged.get(name);
    assert.ok(value !== undefined, `${name} is defined`);
    assert.ok(depth < 8, `${name} has no reference cycle`);
    const ref = value.match(/^var\((--[\w-]+)\)$/);
    return ref ? resolve(ref[1], depth + 1) : value;
  };
  return (token: string) => {
    const hex = resolve(`--g-${token}`);
    assert.match(hex, /^#[0-9a-f]{6}$/i, `--g-${token} resolves to a hex colour, got ${hex}`);
    return hex;
  };
};

const channel = (hex: string, offset: number) => {
  const c = parseInt(hex.slice(1 + offset, 3 + offset), 16) / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};
const luminance = (hex: string) => 0.2126 * channel(hex, 0) + 0.7152 * channel(hex, 2) + 0.0722 * channel(hex, 4);
const contrast = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

const AGENT_ROLES = ["executor", "analyst", "researcher", "reviewer", "tester", "documenter", "planner", "orchestrator", "project-manager"];

// [foreground, background, minimum ratio]: 4.5 for body text, 3 for non-text UI such as outlines and focus rings.
const PAIRS: [string, string, number][] = [
  ["text", "bg", 7],
  ["text", "surface", 7],
  ["text", "surface-raised", 7],
  ["text", "accent-soft", 7],
  ["text", "surface-hover", 7],
  ["text", "surface-active", 7],
  ["text", "board-card-bg", 7],
  ["text-2", "board-card-bg", 4.5],
  ["text-muted", "board-card-bg", 4.5],
  ["text-2", "board-label-bg", 4.5],
  ["accent-soft-text", "board-card-bg", 4.5],
  ["signal-text", "board-card-bg", 4.5],
  ...["maybe", "decision", "prep", "queue", "work", "done"].flatMap((lane) => [
    [`board-lane-${lane}`, "surface", 4.5],
    [`board-lane-${lane}`, "board-card-bg", 4.5],
  ] as [string, string, number][]),
  ["text-2", "bg", 4.5],
  ["text-2", "surface", 4.5],
  ["text-2", "surface-raised", 4.5],
  ["text-2", "surface-hover", 4.5],
  ["text-2", "surface-active", 4.5],
  ["text-muted", "bg", 4.5],
  ["text-muted", "surface", 4.5],
  ["text-muted", "surface-hover", 4.5],
  ["text-muted", "surface-active", 4.5],
  ["accent-text", "bg", 4.5],
  ["accent-text", "surface", 4.5],
  // accent-text is the plain-background step: on a raised or soft field it drops to 4.29:1 (ink) and
  // 4.36:1 (paper), so text there uses accent-soft-text instead.
  ["accent-soft-text", "accent-soft", 4.5],
  ["accent-soft-text", "surface-raised", 4.5],
  ["accent-soft-text", "surface-hover", 4.5],
  ["accent-hover", "bg", 4.5],
  ["accent-hover", "surface", 4.5],
  ["signal-text", "bg", 4.5],
  ["signal-text", "surface", 4.5],
  ["signal-text", "signal-soft", 4.5],
  ["danger-text", "bg", 4.5],
  ["danger-text", "surface", 4.5],
  ["danger-text", "danger-soft", 4.5],
  ["on-accent", "accent", 4.5],
  ["on-signal", "signal", 4.5],
  ["on-signal", "signal-hover", 4.5],
  ["on-danger", "danger", 4.5],
  ["control-border", "bg", 3],
  ["control-border", "surface", 3],
  ["control-border", "surface-raised", 3],
  ["progress-early", "bg", 3],
  ["focus", "bg", 3],
  ["focus", "surface", 3],
  ["focus", "surface-raised", 3],
  ...[1, 2, 3, 4].flatMap((n) => [[`viz-${n}`, "bg", 3], [`viz-${n}`, "surface", 3], [`viz-${n}`, "surface-raised", 3]] as [string, string, number][]),
  ...AGENT_ROLES.map((role) => ["on-agent", `agent-${role}`, 4.5] as [string, string, number]),
];

for (const [name, selector] of [["ink", ":root"], ["paper", '[data-theme="paper"]']] as const) {
  test(`${name} theme meets the documented contrast ratios`, () => {
    const token = theme(selector);
    const failures = PAIRS.flatMap(([fg, bg, min]) => {
      const ratio = contrast(token(fg), token(bg));
      return ratio >= min ? [] : [`--g-${fg} on --g-${bg}: ${ratio.toFixed(2)} < ${min}`];
    });
    assert.deepEqual(failures, []);
  });
}

test("brand orange is never the text colour on paper", () => {
  // 2.12:1 on #f4f1ea; text on paper must use the darker orange-700 step instead.
  const token = theme('[data-theme="paper"]');
  assert.notEqual(token("signal-text").toLowerCase(), token("orange").toLowerCase());
  assert.ok(contrast(token("orange"), token("paper")) < 3);
});

test("the paper theme sets every colour the pairs above depend on", () => {
  // Inheriting an ink-theme colour by accident would pass here only if it happened to contrast; require a conscious choice.
  const paper = declarations('[data-theme="paper"]');
  const used = new Set(PAIRS.flat().filter((v): v is string => typeof v === "string").map((name) => `--g-${name}`));
  const inherited = [...used].filter((name) => !paper.has(name));
  assert.deepEqual(inherited, []);
});

// The raw palette (ink, paper, green-400 …) is not paired directly: it is what the semantic layer above
// names. A divider is decorative by definition — it separates, it does not carry meaning — so it owes no
// ratio. Every other colour token must appear in a pair, or nothing would notice it drifting below its
// ratio: a token nobody checks is a token that silently breaks.
const PALETTE = /^--g-(ink|paper|green|orange|red|n-|hue-)/;
const DECORATIVE = new Set(["--g-border", "--g-border-strong", "--g-viz-grid"]);

test("every colour token is covered by a contrast pair", () => {
  const root = theme(":root");
  const colour = [...declarations(":root").keys()]
    .filter((name) => name.startsWith("--g-") && !PALETTE.test(name) && !DECORATIVE.has(name))
    .filter((name) => {
      try {
        root(name.slice(4));
        return true;
      } catch {
        // Not a colour (spacing, type, shadows): the pairs say nothing about it.
        return false;
      }
    });
  const used = new Set(PAIRS.flatMap(([fg, bg]) => [`--g-${fg}`, `--g-${bg}`]));
  assert.deepEqual(colour.filter((name) => !used.has(name)), [], "a colour token no contrast pair covers: add a pair for it");
});

// Progress categories must remain distinguishable when the theme resolves their semantic colours.
test("epic progress keeps distinct theme-aware stages", () => {
  for (const selector of [":root", '[data-theme="paper"]']) {
    const color = theme(selector);
    const colors = PROGRESS.map((stage) => {
      const token = stage.color.match(/^var\(--g-([\w-]+)\)$/);
      assert.ok(token, `${stage.id} uses a semantic token`);
      return color(token[1]);
    });
    assert.equal(new Set(colors).size, PROGRESS.length, `${selector} collapses progress stages`);
    assert.ok(contrast(color("progress-early"), color("bg")) >= 3, `${selector} hides the draft stage`);
  }
});

// Agent roles are told apart by colour at a glance: every role has its own, none borrows a semantic one.
test("every standard agent role has its own colour, apart from signal and danger", () => {
  for (const selector of [":root", '[data-theme="paper"]']) {
    const color = theme(selector);
    const colours = AGENT_ROLES.map((role) => color(`agent-${role}`).toLowerCase());
    assert.equal(new Set(colours).size, AGENT_ROLES.length, `${selector} gives two roles one colour`);
    for (const semantic of ["signal", "danger", "signal-text", "danger-text"]) assert.ok(!colours.includes(color(semantic).toLowerCase()), `${selector}: a role uses --g-${semantic}`);
  }
});
