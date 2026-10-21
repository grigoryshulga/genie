import { chromium } from "@playwright/test";

const ports = { base: 7452, mine: 7451 };
const route = process.argv[2];

const out = {};
for (const [name, port] of Object.entries(ports)) {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: Number(process.env.W ?? 1440), height: 900 } });
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.getByLabel("Логин").fill("owner");
  await page.getByLabel("Пароль").fill("ui-tests-password");
  await page.getByRole("button", { name: "Войти" }).click();
  await page.waitForTimeout(1500);
  await page.goto(`http://127.0.0.1:${port}${route}`);
  await page.waitForTimeout(2500);
  out[name] = await page.evaluate(() => {
    const h2 = [...document.querySelectorAll("h2")].find((e) => e.textContent.trim() === "Шаги");
    const info = (e) => { if (!e) return null; const c = getComputedStyle(e); const r = e.getBoundingClientRect(); return { cls: e.className, x: Math.round(r.x), w: Math.round(r.width), fs: c.fontSize, lh: c.lineHeight, mt: c.marginTop, mb: c.marginBottom, pl: c.paddingLeft, maxW: c.maxWidth }; };
    return {
      sheets: [...document.styleSheets].map((s) => (s.href || "inline").replace(/.*\/assets\//, "")),
      h2: info(h2),
      chain: h2 ? [h2.parentElement, h2.parentElement?.parentElement, h2.parentElement?.parentElement?.parentElement].map(info) : null,
      body: document.body.innerText.slice(0, 120),
    };
  });
  await browser.close();
}
console.log(JSON.stringify(out, null, 1));
