import { chromium } from "@playwright/test";

const targets = [
  { name: "base", url: "http://127.0.0.1:7452" },
  { name: "mine", url: "http://127.0.0.1:7451" },
];
const routes = process.argv.slice(2).length ? process.argv.slice(2) : ["/automations"];
const width = Number(process.env.W ?? 390);
const height = Number(process.env.H ?? 844);

const browser = await chromium.launch();
const out = {};
for (const t of targets) {
  const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const page = await context.newPage();
  await page.goto(`${t.url}/`, { waitUntil: "load" });
  await page.getByLabel("Логин").fill("owner");
  await page.getByLabel("Пароль").fill("ui-tests-password");
  await page.getByRole("button", { name: "Войти" }).click();
  await page.getByRole("button", { name: "Войти" }).waitFor({ state: "hidden", timeout: 15000 });
  for (const route of routes) {
    await page.goto(`${t.url}${route}`, { waitUntil: "load" });
    await page.waitForTimeout(700);
    out[`${t.name} ${route}`] = await page.evaluate(() => {
      const pick = (sel) => {
        const el = document.querySelector(sel);
        if (!el) return null;
        const b = el.getBoundingClientRect();
        const cs = getComputedStyle(el);
        return { sel, x: +b.x.toFixed(1), y: +b.y.toFixed(1), w: +b.width.toFixed(1), h: +b.height.toFixed(1), display: cs.display, position: cs.position, margin: cs.margin, padding: cs.padding, minH: cs.minHeight, gap: cs.gap };
      };
      const deep = (sel) => {
        const el = document.querySelector(sel);
        if (!el) return null;
        const cs = getComputedStyle(el);
        const pr = el.parentElement ? getComputedStyle(el.parentElement) : null;
        return { sel, display: cs.display, width: cs.width, minWidth: cs.minWidth, maxWidth: cs.maxWidth, flex: cs.flex, alignSelf: cs.alignSelf, boxSizing: cs.boxSizing, position: cs.position, inset: cs.inset,
          parent: pr ? { tag: el.parentElement.tagName + "." + el.parentElement.className, display: pr.display, flexDirection: pr.flexDirection, alignItems: pr.alignItems, padding: pr.padding, width: pr.width, gap: pr.gap, grid: pr.gridTemplateColumns } : null };
      };
      const rules = (() => {
        const el = document.querySelector(".mobile-search-bar");
        const hits = [];
        for (const sheet of document.styleSheets) {
          let list; try { list = sheet.cssRules } catch { continue }
          const walk = (rs, media) => {
            for (const r of rs) {
              if (r.cssRules && r.conditionText !== undefined) { walk(r.cssRules, r.conditionText); continue }
              if (!r.selectorText) continue;
              let m = false; try { m = el.matches(r.selectorText) } catch {}
              if (!m) continue;
              const w = r.style?.width || r.style?.flex || r.style?.margin;
              if (w) hits.push({ media: media ?? null, sel: r.selectorText, width: r.style.width, flex: r.style.flex, margin: r.style.margin });
            }
          };
          walk(list, null);
        }
        return hits;
      })();
      return {
        rules,
        deep: [".mobile-search-bar", ".main", ".split-main", ".pane-body"].map(deep),
        innerW: innerWidth,
        scrollW: document.documentElement.scrollWidth,
        scrollH: document.documentElement.scrollHeight,
        els: [".app", ".main", ".topbar", ".split", ".split-list", ".split-main", ".pane-head", ".pane-body", ".mobile-search-bar"].map(pick),
      };
    });
  }
  await context.close();
}
await browser.close();
console.log(JSON.stringify(out, null, 1));
