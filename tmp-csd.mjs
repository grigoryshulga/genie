import { chromium } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { CAPTURE_ROUTES } from "/work/test/ui/routes.ts";

const ports = { base: 7452, mine: 7451 };
const props = ["display","position","width","height","marginTop","marginRight","marginBottom","marginLeft","paddingTop","paddingRight","paddingBottom","paddingLeft","fontSize","fontWeight","fontFamily","lineHeight","color","backgroundColor","borderTopWidth","borderTopColor","borderRadius","gap","gridTemplateColumns","flexDirection","overflowX","overflowY","textAlign","maxWidth","minWidth","opacity","visibility","transform","letterSpacing","whiteSpace","textTransform"];
const viewports = [{ w: 1440, h: 900 }, { w: 390, h: 844 }];

const result = {};

for (const [name, port] of Object.entries(ports)) {
  const browser = await chromium.launch();
  for (const vp of viewports) {
    const page = await browser.newPage({ viewport: { width: vp.w, height: vp.h } });
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.getByLabel("Логин").fill("owner");
    await page.getByLabel("Пароль").fill("ui-tests-password");
    await page.getByRole("button", { name: "Войти" }).click();
    await page.waitForTimeout(1800);
    for (const route of CAPTURE_ROUTES) {
      await page.goto(`http://127.0.0.1:${port}${route.path}`);
      await page.waitForTimeout(1200);
      const data = await page.evaluate((props) => {
        const sheets = [...document.styleSheets].map((s) => (s.href || "inline").replace(/.*\/assets\//, ""));
        const nodes = [...document.querySelectorAll("body *")].map((el) => {
          const c = getComputedStyle(el);
          const r = el.getBoundingClientRect();
          return { tag: el.tagName.toLowerCase(), cls: typeof el.className === "string" ? el.className : "", rect: [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)].join(","), vals: props.map((p) => c[p]).join("|") };
        });
        return { sheets, nodes };
      }, props);
      result[`${route.name}#${vp.w}#${name}`] = data;
    }
    await page.close();
  }
  await browser.close();
}

const lines = [];
for (const route of CAPTURE_ROUTES) {
  for (const vp of viewports) {
    const a = result[`${route.name}#${vp.w}#base`], b = result[`${route.name}#${vp.w}#mine`];
    if (!a || !b) { lines.push(`${route.name}@${vp.w}: MISSING`); continue; }
    const groups = {};
    const n = Math.min(a.nodes.length, b.nodes.length);
    for (let i = 0; i < n; i++) {
      const x = a.nodes[i], y = b.nodes[i];
      if (x.vals === y.vals && x.rect === y.rect) continue;
      const key = `${x.tag}.${x.cls}`;
      const av = x.vals.split("|"), bv = y.vals.split("|");
      const d = props.filter((_, k) => av[k] !== bv[k]).map((p, k) => p);
      const detail = props.map((p, k) => (av[k] !== bv[k] ? `${p}:${av[k]}→${bv[k]}` : null)).filter(Boolean).slice(0, 6);
      if (x.rect !== y.rect) detail.push(`rect`);
      (groups[key] ??= new Set());
      for (const dd of detail) groups[key].add(dd);
    }
    const keys = Object.keys(groups);
    lines.push(`\n### ${route.name} @${vp.w} — ${keys.length} element groups; nodes ${a.nodes.length}/${b.nodes.length}`);
    lines.push(`  sheets base: ${a.sheets.join(",")}`);
    lines.push(`  sheets mine: ${b.sheets.join(",")}`);
    for (const k of keys.slice(0, 12)) lines.push(`  ${k}: ${[...groups[k]].slice(0, 6).join(" ; ")}`);
  }
}
writeFileSync("/work/tmp-csd.txt", lines.join("\n"));
console.log("done", lines.length);
