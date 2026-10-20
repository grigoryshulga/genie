// Generates the animated Genie character: one standalone SVG per action plus a preview page.
//   node docs/assets/brand/character/build.mjs
//
// The character is the logo mark. Its orange arc is the eye: scaleY(1) is the open "∩" of the logo, scaleY(0) a
// closed line, scaleY(-1) a sleeping "∪". The handle is an antenna that reacts independently of the head.
// Geometry is the draft mark from ../logo-mark.svg, shifted by (DX, DY) so every transform pivot below is a
// plain coordinate in a viewBox that starts at 0,0 (transform-box: view-box resolves against that origin).

import { writeFileSync } from "node:fs";
import path from "node:path";

const OUT = import.meta.dirname;
const W = 880;
const H = 780;
const DX = -20;
const DY = 150;
const x = (v) => v + DX;
const y = (v) => v + DY;

const EYE = { x: x(315), y: y(233) }; // centre of the arc's circle
const NECK = { x: x(300), y: y(400) }; // pivot for head tilt: bottom of the ring
const HANDLE = { x: x(452), y: y(152) }; // where the antenna meets the ring
const FEET = { x: x(290), y: y(600) }; // pivot for squash and stretch: bottom of the body

const deg = (n) => `rotate(${n}deg)`;
const squash = (sx, sy, ty = 0) => `translate(0,${ty}px) scale(${sx},${sy})`;

// Each action is a keyframe table per part: { part: { percent: transform | {transform, opacity} } }.
// Every part is at its neutral (logo) pose in the 0% frame unless the action starts in a different pose.
const ACTIONS = {
  surprise: {
    title: "Surprise",
    desc: "The character crouches, jumps with its eye wide open as a full ring, lands, and trembles.",
    duration: 3.4,
    frames: {
      all: { 0: squash(1, 1), 8: squash(1.05, 0.93), 17: squash(0.96, 1.06, -36), 30: squash(1.08, 0.9), 40: squash(0.98, 1.03), 48: squash(1, 1), 100: squash(1, 1) },
      head: { 0: deg(0), 17: deg(-4), 30: deg(2), 48: deg(0), 52: deg(-1.5), 56: deg(1.5), 60: deg(-1.5), 64: deg(1.5), 68: deg(0), 100: deg(0) },
      handle: { 0: deg(0), 17: deg(-38), 30: deg(16), 40: deg(-12), 50: deg(0), 54: deg(-5), 58: deg(5), 62: deg(-4), 66: deg(0), 100: deg(0) },
      eye: { 0: "scale(1)", 8: "scale(.8)", 17: "scale(1.28)", 30: "scale(1.1)", 48: "scale(1.18)", 82: "scale(1.18)", 92: "scale(1)", 100: "scale(1)" },
      lt: { 0: "scaleY(1)", 8: "scaleY(.75)", 17: "scaleY(1)", 100: "scaleY(1)" },
      lb: { 0: "scaleY(0)", 10: "scaleY(0)", 17: "scaleY(1)", 82: "scaleY(1)", 92: "scaleY(0)", 100: "scaleY(0)" },
      sparks: { 0: { opacity: 0, transform: "scale(.7)" }, 16: { opacity: 0, transform: "scale(.7)" }, 22: { opacity: 1, transform: "scale(1)" }, 40: { opacity: 1, transform: "scale(1.15)" }, 52: { opacity: 0, transform: "scale(1.2)" }, 100: { opacity: 0, transform: "scale(1.2)" } },
    },
  },
  wake: {
    title: "Waking up",
    desc: "Asleep with a drooping antenna, the character stirs, stretches, opens its eye wide, blinks, and drifts back off.",
    duration: 7,
    frames: {
      // Asleep: eye is a closed "∪", head dropped, antenna hanging, body slumped.
      all: { 0: squash(1.02, 0.95), 8: squash(1.015, 0.965), 16: squash(1.02, 0.95), 24: squash(1.02, 0.95), 44: squash(1, 0.97), 52: squash(0.95, 1.1, -14), 62: squash(1.04, 0.96), 70: squash(1, 1), 88: squash(1, 1), 100: squash(1.02, 0.95) },
      head: { 0: deg(6), 36: deg(5), 46: deg(0), 52: deg(-5), 62: deg(2), 70: deg(0), 88: deg(0), 100: deg(6) },
      handle: { 0: deg(55), 44: deg(45), 52: deg(-30), 60: deg(10), 66: deg(-6), 72: deg(0), 88: deg(0), 100: deg(55) },
      eye: { 0: "scale(1)", 100: "scale(1)" },
      lt: { 0: "scaleY(-1)", 24: "scaleY(-1)", 26: "scaleY(-.15)", 30: "scaleY(-1)", 44: "scaleY(.08)", 52: "scaleY(.6)", 56: "scaleY(1.25)", 62: "scaleY(1)", 80: "scaleY(1)", 83: "scaleY(.08)", 86: "scaleY(1)", 90: "scaleY(1)", 100: "scaleY(-1)" },
      lb: { 0: "scaleY(0)", 100: "scaleY(0)" },
      z1: floatingZ(2, 20),
      z2: floatingZ(8, 26),
      z3: floatingZ(14, 28),
    },
  },
  sleep: {
    title: "Falling asleep",
    desc: "The character blinks heavily, droops, closes its eye into a sleeping curve, snores softly, then startles awake.",
    duration: 8,
    frames: {
      all: { 0: squash(1, 1), 48: squash(1.02, 0.95), 56: squash(1.015, 0.965), 64: squash(1.02, 0.95), 72: squash(1.015, 0.965), 82: squash(1.02, 0.95), 88: squash(0.97, 1.06, -10), 94: squash(1, 1), 100: squash(1, 1) },
      head: { 0: deg(0), 14: deg(2), 26: deg(3), 32: deg(1), 40: deg(5), 48: deg(7), 82: deg(7), 88: deg(-3), 94: deg(0), 100: deg(0) },
      handle: { 0: deg(0), 40: deg(25), 48: deg(58), 82: deg(58), 88: deg(-20), 94: deg(0), 100: deg(0) },
      eye: { 0: "scale(1)", 100: "scale(1)" },
      lt: { 0: "scaleY(1)", 8: "scaleY(1)", 12: "scaleY(.1)", 16: "scaleY(.85)", 26: "scaleY(.08)", 32: "scaleY(.55)", 40: "scaleY(.08)", 46: "scaleY(-1.1)", 50: "scaleY(-1)", 82: "scaleY(-1)", 88: "scaleY(-.2)", 94: "scaleY(1)", 100: "scaleY(1)" },
      lb: { 0: "scaleY(0)", 100: "scaleY(0)" },
      z1: floatingZ(52, 22),
      z2: floatingZ(60, 22),
      z3: floatingZ(68, 20),
    },
  },
  think: {
    title: "Thinking",
    desc: "The character looks up and to the side, glances across, squints, and a trail of thought bubbles appears.",
    duration: 6.5,
    frames: {
      all: { 0: "rotate(0deg)", 12: deg(-1.5), 40: deg(-1.5), 46: deg(1), 62: deg(1), 70: deg(-1.5), 86: deg(-1.5), 94: deg(0), 100: deg(0) },
      head: { 0: deg(0), 12: deg(-5), 40: deg(-5), 46: deg(4), 62: deg(4), 70: deg(-5), 86: deg(-5), 94: deg(0), 100: deg(0) },
      handle: { 0: deg(0), 12: deg(-14), 40: deg(-14), 46: deg(8), 62: deg(8), 70: deg(-14), 86: deg(-14), 94: deg(0), 100: deg(0) },
      eye: { 0: "translate(0,0) rotate(0deg)", 12: "translate(26px,-24px) rotate(-12deg)", 40: "translate(26px,-24px) rotate(-12deg)", 46: "translate(-26px,-18px) rotate(12deg)", 62: "translate(-26px,-18px) rotate(12deg)", 70: "translate(26px,-24px) rotate(-12deg)", 86: "translate(26px,-24px) rotate(-12deg)", 94: "translate(0,0) rotate(0deg)", 100: "translate(0,0) rotate(0deg)" },
      lt: { 0: "scaleY(1)", 12: "scaleY(.8)", 76: "scaleY(.8)", 79: "scaleY(.08)", 82: "scaleY(.8)", 86: "scaleY(.8)", 94: "scaleY(1)", 100: "scaleY(1)" },
      lb: { 0: "scaleY(0)", 100: "scaleY(0)" },
      d1: popIn(16, 80),
      d2: popIn(22, 80),
      d3: popIn(28, 80),
      dot1: blink(36),
      dot2: blink(41),
      dot3: blink(46),
    },
  },
};

function floatingZ(start, span) {
  const end = start + span;
  const hidden = { opacity: 0, transform: "translate(0,0) scale(.7)" };
  return { 0: hidden, [start]: hidden, [start + span * 0.25]: { opacity: 0.95, transform: "translate(8px,-14px) scale(.9)" }, [end]: { opacity: 0, transform: "translate(26px,-60px) scale(1.1)" }, 100: { opacity: 0, transform: "translate(26px,-60px) scale(1.1)" } };
}
function popIn(start, end) {
  const hidden = { opacity: 0, transform: "scale(0)" };
  return { 0: hidden, [start]: hidden, [start + 4]: { opacity: 1, transform: "scale(1.2)" }, [start + 7]: { opacity: 1, transform: "scale(1)" }, [end]: { opacity: 1, transform: "scale(1)" }, [end + 6]: hidden, 100: hidden };
}
function blink(start) {
  return { 0: { opacity: 0.25 }, [start]: { opacity: 0.25 }, [start + 3]: { opacity: 1 }, [start + 9]: { opacity: 1 }, [start + 12]: { opacity: 0.25 }, 100: { opacity: 0.25 } };
}

// CSS for one action, scoped under its root class so several can be inlined in one document.
function css(name, action) {
  const root = `.a-${name}`;
  const origins = {
    all: FEET, head: NECK, handle: HANDLE, eye: EYE, lt: EYE, lb: EYE,
    sparks: { x: x(314), y: y(229) },
    z1: pt(Z_POS[0]), z2: pt(Z_POS[1]), z3: pt(Z_POS[2]),
    d1: { x: D_POS[0].x, y: D_POS[0].y }, d2: { x: D_POS[1].x, y: D_POS[1].y }, d3: { x: D_POS[2].x, y: D_POS[2].y },
  };
  const rules = [];
  const keyframes = [];
  for (const [part, table] of Object.entries(action.frames)) {
    const o = origins[part];
    const kfName = `${name}-${part}`;
    // dot1..dot3 only pulse opacity inside the bubble, so they need no transform origin.
    rules.push(`${root} .${part} { ${o ? `transform-origin: ${o.x}px ${o.y}px; ` : ""}animation: ${kfName} ${action.duration}s ${EASE} infinite; }`);
    const steps = Object.entries(table).sort((a, b) => a[0] - b[0]).map(([pct, v]) => {
      let decl = typeof v === "string" ? `transform: ${v}` : Object.entries(v).map(([k, val]) => `${k}: ${val}`).join("; ");
      // A closed lid (negative scale) is drawn as a thin curve; thick strokes read as a smile instead of closed eyes.
      if (part === "lt") decl += `; stroke-width: ${Number(/scaleY\((-?[\d.]+)\)/.exec(v)[1]) < 0 ? 16 : 40}px`;
      return `${pct}% { ${decl} }`;
    });
    keyframes.push(`@keyframes ${kfName} { ${steps.join(" ")} }`);
  }
  return [
    `${root} { --g: #008f6b; --o: #ff873c; --p: #f4f1ea; }`,
    `${root} * { transform-box: view-box; }`,
    // The neutral pose is the logo: parts that only exist during the action stay hidden unless animated.
    `${root} .sparks, ${root} .z1, ${root} .z2, ${root} .z3, ${root} .d1, ${root} .d2, ${root} .d3 { opacity: 0; }`,
    `${root} .lb { transform: scaleY(0); transform-origin: ${EYE.x}px ${EYE.y}px; }`,
    ...rules,
    ...keyframes,
    `${root}.paused * { animation-play-state: paused; }`,
    // Reduced motion shows the neutral logo pose instead of a frozen mid-action frame.
    `@media (prefers-reduced-motion: reduce) { ${root} * { animation: none !important; } }`,
  ].join("\n");
}
const EASE = "cubic-bezier(.45,0,.25,1)";

const Z_POS = [{ x: 610, y: 400 }, { x: 665, y: 335 }, { x: 735, y: 255 }];
const Z_SCALE = [0.7, 0.9, 1.15];
const D_POS = [{ x: 632, y: 238 }, { x: 692, y: 180 }, { x: 772, y: 112 }];
function pt(p) { return { x: p.x, y: p.y }; }

const mark = () => `  <g class="all">
    <g class="body" fill="none" stroke="var(--g)">
      <rect x="${x(92)}" y="${y(418)}" width="390" height="144" rx="72" stroke-width="76"/>
      <path d="M${x(190)} ${y(278)}C${x(130)} ${y(268)} ${x(98)} ${y(320)} ${x(112)} ${y(372)}C${x(116)} ${y(390)} ${x(118)} ${y(400)} ${x(120)} ${y(418)}" stroke-width="66"/>
    </g>
    <g class="head" fill="none" stroke="var(--g)">
      <circle cx="${x(314)}" cy="${y(229)}" r="136" stroke-width="70"/>
      <g class="handle"><path d="M${x(456)} ${y(154)}L${x(580)} ${y(103)}" stroke-width="56"/></g>
      <g class="eye" stroke="var(--o)" stroke-width="40">
        <path class="lt" d="M${x(253)} ${y(233.5)}A62 62 0 0 1 ${x(377)} ${y(233.5)}"/>
        <g class="lb">
          <path d="M${x(253)} ${y(232.5)}A62 62 0 0 0 ${x(377)} ${y(232.5)}"/>
          <!-- Opaque patches over the join with the upper half, so the full ring has no antialiasing seam. -->
          <path d="M${x(233)} ${y(228)}H${x(273)}V${y(238)}H${x(233)}ZM${x(357)} ${y(228)}H${x(397)}V${y(238)}H${x(357)}Z" fill="var(--o)" stroke="none"/>
        </g>
      </g>
    </g>
  </g>`;

// Extras that exist only for some actions.
function extras(name) {
  if (name === "surprise") {
    const c = { x: x(314), y: y(229) };
    const lines = [-150, -125, -100, -75].map((a) => {
      const r = (d) => [c.x + Math.cos((a * Math.PI) / 180) * d, c.y + Math.sin((a * Math.PI) / 180) * d].map((v) => v.toFixed(1));
      const [x1, y1] = r(196);
      const [x2, y2] = r(244);
      return `<path d="M${x1} ${y1}L${x2} ${y2}"/>`;
    });
    return `  <g class="sparks" fill="none" stroke="var(--o)" stroke-width="14" stroke-linecap="round">${lines.join("")}</g>`;
  }
  if (name === "wake" || name === "sleep") {
    return Z_POS.map((p, i) => `  <g class="z${i + 1}"><path transform="translate(${p.x} ${p.y}) scale(${Z_SCALE[i]})" d="M0 0H40L0 52H40" fill="none" stroke="var(--p)" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/></g>`).join("\n");
  }
  if (name === "think") {
    const dots = [12, 19].map((r, i) => `  <g class="d${i + 1}"><circle cx="${D_POS[i].x}" cy="${D_POS[i].y}" r="${r}" fill="var(--o)"/></g>`);
    const b = D_POS[2];
    const bubble = `  <g class="d3"><circle cx="${b.x}" cy="${b.y}" r="60" fill="none" stroke="var(--o)" stroke-width="10"/>${[-26, 0, 26].map((dx, i) => `<circle class="dot${i + 1}" cx="${b.x + dx}" cy="${b.y}" r="9" fill="var(--o)"/>`).join("")}</g>`;
    return [...dots, bubble].join("\n");
  }
  return "";
}

function svg(name, action) {
  return `<svg xmlns="http://www.w3.org/2000/svg" class="a-${name}" viewBox="0 0 ${W} ${H}" fill="none">
  <title>${action.title}</title>
  <desc>${action.desc}</desc>
  <style>
${css(name, action)}
  </style>
${mark()}
${extras(name)}
</svg>
`;
}

const markup = {};
for (const [name, action] of Object.entries(ACTIONS)) {
  markup[name] = svg(name, action);
  writeFileSync(path.join(OUT, `genie-${name}.svg`), markup[name]);
}

const cards = Object.entries(ACTIONS).map(([name, a]) => `    <article class="g-card">
      <div class="stage">${markup[name].replace(/<\?xml[^>]*>/, "").replace(' xmlns="http://www.w3.org/2000/svg"', "")}</div>
      <h3 class="t-h4">${a.title}</h3>
      <p class="t-caption">${a.desc}</p>
      <div class="row"><button class="g-btn secondary sm" data-replay="${name}" type="button">Replay</button><button class="g-btn ghost sm" data-pause="${name}" type="button" aria-pressed="false">Pause</button></div>
    </article>`).join("\n");

writeFileSync(path.join(OUT, "animations.html"), `<!doctype html>
<html lang="en" data-theme="ink">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Genie character animations</title>
<!-- Generated by build.mjs. Open from a checkout; it reads the real design tokens. -->
<link rel="stylesheet" href="../../../../web/src/shared/ui/tokens.css">
<style>
* { box-sizing: border-box; }
body { margin: 0; background: var(--g-bg); color: var(--g-text); font: 400 var(--g-text-body)/var(--g-leading-prose) var(--g-font-sans); -webkit-font-smoothing: antialiased; }
:focus-visible { outline: 2px solid var(--g-focus); outline-offset: 3px; }
.wrap { max-width: 1120px; margin: 0 auto; padding: 0 var(--g-space-5) var(--g-space-8); }
header { display: flex; align-items: center; gap: var(--g-space-4); padding: var(--g-space-4) 0; border-bottom: 1px solid var(--g-border); }
header .grow { flex: 1; }
h1 { font-family: var(--g-font-display); font-weight: 900; font-size: var(--g-text-h1); letter-spacing: var(--g-tracking-heading); line-height: var(--g-leading-heading); margin: var(--g-space-7) 0 var(--g-space-3); }
.lead { max-width: 60ch; color: var(--g-text-2); margin: 0 0 var(--g-space-6); }
.grid { display: grid; gap: var(--g-space-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 420px), 1fr)); }
.g-card { padding: var(--g-space-5); border: 1px solid var(--g-border); border-radius: var(--g-radius-lg); background: var(--g-surface); }
.stage { aspect-ratio: 880 / 780; margin-bottom: var(--g-space-4); border-radius: var(--g-radius-md); background: var(--g-bg); overflow: hidden; }
.stage svg { display: block; width: 100%; height: 100%; }
.t-h4 { margin: 0 0 var(--g-space-1); font-size: var(--g-text-h4); font-weight: 600; }
.t-caption { margin: 0 0 var(--g-space-4); font-size: var(--g-text-caption); font-weight: 500; color: var(--g-text-muted); line-height: 1.5; }
.row { display: flex; gap: var(--g-space-2); }
.g-btn { display: inline-flex; align-items: center; justify-content: center; height: 40px; padding: 0 var(--g-space-5); border: 1.5px solid transparent; border-radius: var(--g-radius-pill); background: transparent; color: var(--g-text); font: 600 var(--g-text-ui)/1 var(--g-font-sans); cursor: pointer; }
.g-btn.sm { height: 32px; padding: 0 var(--g-space-4); font-size: var(--g-text-dense); }
.g-btn.secondary { border-color: var(--g-accent); }
.g-btn.secondary:hover { background: var(--g-accent-soft); }
.g-btn.ghost { color: var(--g-text-2); }
.g-btn.ghost:hover { background: var(--g-surface-hover); color: var(--g-text); }
/* On paper the mark steps down to the deep green and the "z" marks turn ink, as in the logo variants. */
[data-theme="paper"] .stage svg { --g: #00604a; --p: #09090d; }
</style>
</head>
<body>
<div class="wrap">
  <header><strong>genie</strong><span class="grow"></span><button class="g-btn secondary sm" id="theme" type="button" aria-pressed="false">Light theme</button></header>
  <h1>Character animations</h1>
  <p class="lead">The logo mark as a character. The orange arc is its eye and the handle is an antenna. Each action loops and returns to the logo pose. With reduced motion enabled, only the logo pose is shown.</p>
  <div class="grid">
${cards}
  </div>
</div>
<script>
const root = document.documentElement;
const theme = document.getElementById('theme');
theme.addEventListener('click', () => {
  const paper = root.dataset.theme !== 'paper';
  root.dataset.theme = paper ? 'paper' : 'ink';
  theme.textContent = paper ? 'Dark theme' : 'Light theme';
  theme.setAttribute('aria-pressed', String(paper));
});
const svgOf = (name) => document.querySelector('.a-' + name);
document.querySelectorAll('[data-replay]').forEach((b) => b.addEventListener('click', () => {
  const s = svgOf(b.dataset.replay);
  s.classList.remove('paused');
  s.getAnimations({ subtree: true }).forEach((a) => { a.currentTime = 0; a.play(); });
}));
document.querySelectorAll('[data-pause]').forEach((b) => b.addEventListener('click', () => {
  const paused = svgOf(b.dataset.pause).classList.toggle('paused');
  b.textContent = paused ? 'Play' : 'Pause';
  b.setAttribute('aria-pressed', String(paused));
}));
</script>
</body>
</html>
`);
console.log("wrote", Object.keys(ACTIONS).map((n) => `genie-${n}.svg`).join(", "), "and animations.html");
