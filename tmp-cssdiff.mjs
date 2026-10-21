import { chromium } from "@playwright/test";
import { writeFileSync } from "node:fs";

const ports = { base: 7452, mine: 7451 };
const route = process.argv[2];
const W = Number(process.env.W ?? 1440);
const H = Number(process.env.H ?? 900);

const props = ["display","position","width","height","marginTop","marginRight","marginBottom","marginLeft","paddingTop","paddingRight","paddingBottom","paddingLeft","fontSize","fontWeight","fontFamily","lineHeight","color","backgroundColor","borderTopWidth","borderTopColor","borderRadius","gap","gridTemplateColumns","flexDirection","overflowX","overflowY","textAlign","maxWidth","minWidth","opacity","visibility","transform","letterSpacing","whiteSpace","textTransform"];

const collected = {};
for (const [name, port] of Object.entries(ports)) {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: W, height: H } });
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.getByLabel("Логин").fill("owner");
  await page.getByLabel("Пароль").fill("ui-tests-password");
  await page.getByRole("button", { name: "Войти" }).click();
  await page.waitForTimeout(1500);
  await page.goto(`http://127.0.0.1:${port}${route}`);
  await page.waitForTimeout(2500);
  const meta = await page.evaluate(() => ({
    sheets: [...document.styleSheets].map((s) => (s.href || "inline").replace(/.*\/assets\//, "")),
    text: document.body.innerText.length,
  }));
  collected[name] = meta;
  collected[name + "#nodes"] = await page.evaluate((props) => {
    const nodes = [...document.querySelectorAll("body *")];
    return nodes.map((el, i) => {
      const c = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      const vals = props.map((p) => c[p]);
      return { i, tag: el.tagName.toLowerCase(), cls: typeof el.className === "string" ? el.className : "", rect: [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)], vals };
    });
  }, props);
  await browser.close();
}

const a = collected["base#nodes"], b = collected["mine#nodes"];
const lines = [`# ${route} @${W}x${H}`, `sheets base=${collected.base.sheets.join(",")}`, `sheets mine=${collected.mine.sheets.join(",")}`, `textLen base=${collected.base.text} mine=${collected.mine.text}`, `nodes base=${a.length} mine=${b.length}`];
if (a.length !== b.length) lines.push("!! node count differs");
const byCls = {};
for (let i = 0; i < Math.min(a.length, b.length); i++) {
  const x = a[i], y = b[i];
  const diffs = [];
  props.forEach((p, k) => { if (x.vals[k] !== y.vals[k]) diffs.push(`${p}: ${x.vals[k]} -> ${y.vals[k]}`); });
  if (x.rect.join() !== y.rect.join()) diffs.push(`rect: ${x.rect.join(",")} -> ${y.rect.join(",")}`);
  if (!diffs.length) continue;
  const key = `${x.tag}.${x.cls}`;
  byCls[key] = byCls[key] ?? [];
  if (byCls[key].length < 2) byCls[key].push(diffs.slice(0, 8).join(" | "));
}
for (const [k, v] of Object.entries(byCls)) {
  lines.push(`\n## ${k}`);
  for (const d of v) lines.push("   " + d);
}
writeFileSync(process.env.OUT ?? "/tmp/cssdiff.txt", lines.join("\n"));
console.log(`groups: ${Object.keys(byCls).length} -> ${process.env.OUT ?? "/tmp/cssdiff.txt"}`);
