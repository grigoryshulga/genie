// Accessibility rules the redesign promises, checked against a page that is actually rendered.
//
// This is a small rule engine that runs inside the browser from the UI harness rather than a
// third-party auditor: the repository keeps test-only tooling out of the product dependency graph
// (the brand handoff's open decision 4), and the checks below are the promises the design system
// itself makes — named controls, sound roles, readable contrast, focus that is visible and stays
// inside a dialog, and motion that stops when the person asked for less of it. What it does not
// check is written down beside the harness in docs/development.md.

export type Severity = "critical" | "serious" | "moderate";

export interface Finding {
  rule: string;
  severity: Severity;
  /** The element as a short, greppable path: `button.g-btn.primary#save`. */
  target: string;
  detail: string;
}

export interface AuditResult {
  findings: Finding[];
  /** How many elements and text runs the rules looked at, so a green run says what it saw. */
  checked: number;
}

export interface MotionReport {
  /** Animations still running when the page settled: under reduced motion there should be none. */
  animations: string[];
  /** Elements whose transition still moves something (transform, size, position) under reduced motion. */
  transitions: string[];
}

// The whole audit lives in one function because page.evaluate serialises it into the browser: it
// cannot close over module scope, so the helpers are nested inside it.
export function auditPage(): AuditResult {
  const findings: Finding[] = [];
  const seen = new Set<string>();
  let checked = 0;

  const label = (el: Element): string => {
    const tag = el.tagName.toLowerCase();
    const id = el.id ? `#${el.id}` : "";
    const cls = (el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2);
    return tag + id + cls.map((c) => `.${c}`).join("");
  };

  const report = (rule: string, severity: Severity, el: Element | null, detail: string) => {
    const target = el ? label(el) : "document";
    const key = `${rule}|${target}|${detail}`;
    if (seen.has(key)) return;
    seen.add(key);
    findings.push({ rule, severity, target, detail });
  };

  const isHidden = (el: Element): boolean => {
    if (el.closest('[aria-hidden="true"], [inert], [hidden]')) return true;
    const s = getComputedStyle(el);
    if (s.display === "none" || s.visibility === "hidden" || s.visibility === "collapse") return true;
    return el.getClientRects().length === 0;
  };

  // The accessible name as a browser would derive it, simplified: aria-label, aria-labelledby, a
  // <label>, the value of a button input, the text content, then alt or title.
  const name = (el: Element): string => {
    const aria = el.getAttribute("aria-label");
    if (aria && aria.trim()) return aria.trim();
    const by = el.getAttribute("aria-labelledby");
    if (by) {
      const text = by
        .split(/\s+/)
        .map((id) => document.getElementById(id)?.textContent?.trim() ?? "")
        .join(" ")
        .trim();
      if (text) return text;
    }
    const labels = (el as HTMLInputElement).labels;
    if (labels && labels.length > 0) {
      const text = [...labels].map((l) => l.textContent ?? "").join(" ").trim();
      if (text) return text;
    }
    if (el instanceof HTMLInputElement && ["submit", "button", "reset"].includes(el.type) && el.value.trim()) return el.value.trim();
    const text = (el.textContent ?? "").trim();
    if (text) return text;
    const alt = el.getAttribute("alt");
    if (alt && alt.trim()) return alt.trim();
    const title = el.getAttribute("title");
    return title ? title.trim() : "";
  };

  const INTERACTIVE =
    'a[href], button, input:not([type="hidden"]), select, textarea, summary, [role="button"], [role="link"], ' +
    '[role="checkbox"], [role="radio"], [role="tab"], [role="switch"], [role="menuitem"], [role="menuitemcheckbox"], ' +
    '[role="combobox"], [role="searchbox"], [role="textbox"], [contenteditable="true"]';

  const FOCUSABLE =
    'a[href], button, input:not([type="hidden"]), select, textarea, iframe, [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

  const all = [...document.querySelectorAll("*")];

  // --- names, roles, structure ----------------------------------------------------------------
  let mains = 0;
  let headingLevel = 0;
  for (const el of all) {
    if (el.matches(INTERACTIVE) && !isHidden(el)) {
      checked += 1;
      if (el.getAttribute("aria-disabled") !== "true" && !name(el)) {
        report("control-name", "serious", el, "an interactive element has no accessible name");
      }
      if (el.matches("a a, a button, button a, button button, button summary")) {
        report("nested-interactive", "serious", el, "an interactive element is nested inside another one");
      }
    }

    if (el.matches("img:not([alt])")) report("image-alt", "critical", el, "an image has no alt attribute");

    // An icon that stands on its own carries no text of its own, so only a name can identify it.
    if (el.matches("svg") && !isHidden(el) && !el.closest(INTERACTIVE) && el.getAttribute("aria-hidden") !== "true" && el.getAttribute("role") !== "img" && !el.querySelector("title")) {
      report("svg-name", "moderate", el, "a standalone icon is neither hidden from assistive technology nor named");
    }

    if (el.matches('[aria-modal="true"]') && !name(el)) {
      report("modal-name", "serious", el, "a modal dialog has no accessible name");
    }

    if (el.matches("[tabindex]")) {
      const index = Number(el.getAttribute("tabindex"));
      if (Number.isFinite(index) && index > 0) report("positive-tabindex", "serious", el, `tabindex="${index}" puts this element before the page in tab order`);
    }

    if (el.matches('[aria-hidden="true"]') && el.querySelector(FOCUSABLE)) {
      report("aria-hidden-focus", "serious", el, "a focusable element sits inside aria-hidden, so it is reachable by keyboard but invisible to a screen reader");
    }

    if (el.matches("main")) mains += 1;

    const heading = el.matches("h1, h2, h3, h4, h5, h6") ? Number(el.tagName.slice(1)) : 0;
    if (heading && !isHidden(el)) {
      if (headingLevel && heading > headingLevel + 1) {
        report("heading-order", "moderate", el, `a level ${heading} heading follows a level ${headingLevel} one, skipping a level`);
      }
      headingLevel = heading;
    }
  }
  if (mains !== 1) report("main-landmark", "moderate", null, `the page has ${mains} main landmarks instead of one`);

  const ids = new Map<string, Element>();
  for (const el of document.querySelectorAll("[id]")) {
    const id = el.id;
    const first = ids.get(id);
    if (first) report("duplicate-id", "serious", el, `the id "${id}" is used by more than one element`);
    else ids.set(id, el);
  }

  // --- targets the pointer can hit -------------------------------------------------------------
  for (const el of all) {
    if (!el.matches(INTERACTIVE) || isHidden(el)) continue;
    const s = getComputedStyle(el);
    // Links inside a sentence are text, not targets: WCAG measures those by the text around them.
    if (el.tagName === "A" && s.display.startsWith("inline")) continue;
    // A control wrapped in its own <label> is hit through the label: that is the target the pointer sees.
    const box = (el.closest("label") ?? el).getBoundingClientRect();
    if (box.width === 0 || box.height === 0) continue;
    if (box.width < 24 || box.height < 24) {
      report("target-size", "moderate", el, `the target is ${Math.round(box.width)}×${Math.round(box.height)}px, below the 24×24px floor`);
    }
  }

  // --- text contrast ---------------------------------------------------------------------------
  const parse = (value: string): { r: number; g: number; b: number; a: number } | undefined => {
    const m = value.match(/rgba?\(([^)]+)\)/);
    if (!m) return undefined;
    const parts = m[1].split(/[,\s/]+/).filter(Boolean).map(Number);
    if (parts.length < 3 || parts.some((n) => !Number.isFinite(n))) return undefined;
    return { r: parts[0], g: parts[1], b: parts[2], a: parts.length > 3 ? parts[3] : 1 };
  };
  const blend = (fg: { r: number; g: number; b: number; a: number }, bg: { r: number; g: number; b: number; a: number }) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });
  // The colour actually behind a run of text: its own background over the nearest opaque ancestor.
  const background = (el: Element) => {
    const layers: { r: number; g: number; b: number; a: number }[] = [];
    for (let node: Element | null = el; node; node = node.parentElement) {
      const c = parse(getComputedStyle(node).backgroundColor);
      if (!c || c.a === 0) continue;
      layers.push(c);
      if (c.a === 1) break;
    }
    let out = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = layers.length - 1; i >= 0; i -= 1) out = blend(layers[i], out);
    return out;
  };
  const lum = (c: { r: number; g: number; b: number }) => {
    const f = (v: number) => {
      const x = v / 255;
      return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
  };
  const ratio = (a: { r: number; g: number; b: number }, b: { r: number; g: number; b: number }) => {
    const [hi, lo] = lum(a) >= lum(b) ? [lum(a), lum(b)] : [lum(b), lum(a)];
    return (hi + 0.05) / (lo + 0.05);
  };

  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.nodeValue?.trim();
    if (!text) continue;
    const parent = node.parentElement;
    if (!parent || isHidden(parent)) continue;
    if (parent.closest("textarea, option, script, style")) continue;
    // The text of an image is its drawing, not copy: a glyph inside role="img" is judged as a graphic
    // (3:1, in the token tests), and its label is what a screen reader reads.
    if (parent.closest('[role="img"], [role="presentation"], [role="none"]')) continue;
    const s = getComputedStyle(parent);
    if (s.contentVisibility === "hidden") continue;
    if (parent.closest(':disabled, [aria-disabled="true"]')) continue;
    const fg = parse(s.color);
    if (!fg || fg.a === 0) continue;
    checked += 1;
    const bg = background(parent);
    const shown = fg.a === 1 ? fg : blend(fg, bg);
    const size = Number.parseFloat(s.fontSize);
    const weight = Number.parseInt(s.fontWeight, 10);
    const large = size >= 24 || (size >= 18.66 && weight >= 700);
    const need = large ? 3 : 4.5;
    const got = ratio(shown, bg);
    if (got + 0.005 < need) {
      report("color-contrast", "serious", parent, `"${text.slice(0, 24)}" is drawn at ${got.toFixed(2)}:1, below the ${need}:1 it needs`);
    }
  }

  return { findings, checked };
}

// Under prefers-reduced-motion nothing essential moves. Two things can still move: an animation that
// is running, and a transition on a property that changes shape or position (colour transitions are
// not motion and are allowed).
export function motionReport(): MotionReport {
  const label = (el: Element | null): string => {
    if (!el) return "document";
    const tag = el.tagName.toLowerCase();
    const id = el.id ? `#${el.id}` : "";
    const cls = (el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2);
    return tag + id + cls.map((c) => `.${c}`).join("");
  };

  const animations: string[] = [];
  for (const animation of document.getAnimations()) {
    if (animation.playState !== "running") continue;
    const target = (animation.effect as unknown as { target?: Element | null } | null)?.target ?? null;
    const name = animation instanceof CSSAnimation ? animation.animationName : `${(animation as CSSTransition).transitionProperty} transition`;
    animations.push(`${label(target)} ${name}`);
  }

  const MOTION = /^(transform|translate|scale|rotate|top|right|bottom|left|width|height|margin|padding|inset|all)$/;
  const transitions: string[] = [];
  for (const el of document.querySelectorAll("*")) {
    const s = getComputedStyle(el);
    if (!s.transitionProperty || s.transitionProperty === "none") continue;
    const properties = s.transitionProperty.split(",").map((p) => p.trim());
    const durations = s.transitionDuration.split(",").map((d) => Number.parseFloat(d));
    const moving = properties.filter((p, i) => MOTION.test(p) && (durations[i % durations.length] ?? 0) > 0);
    if (moving.length > 0) transitions.push(`${label(el)} ${moving.join(", ")}`);
  }

  return { animations, transitions };
}

// Whether the element that has focus right now shows where it is. A focus ring may sit on the control
// itself or on the wrapper that draws the field (an outlined composer around a bare textarea), so a
// couple of ancestors are looked at too.
export function focusVisibility(): { target: string; visible: boolean } {
  const el = document.activeElement;
  if (!el || el === document.body) return { target: "document", visible: false };
  const target = el.tagName.toLowerCase() + (el.id ? `#${el.id}` : "");
  for (let node: Element | null = el, depth = 0; node && depth < 3; node = node.parentElement, depth += 1) {
    const s = getComputedStyle(node);
    const outlined = s.outlineStyle !== "none" && Number.parseFloat(s.outlineWidth) > 0;
    if (outlined || s.boxShadow !== "none") return { target, visible: true };
  }
  return { target, visible: false };
}
