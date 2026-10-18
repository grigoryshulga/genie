/* Paste into DevTools on the rendered Genie page, then await GenieCapture.download(). */
globalThis.GenieCapture = (() => {
  const version = 1;
  async function capture() {
    await document.fonts.ready;
    const warnings = new Set();
    let count = 0;
    const viewport = { x: 0, y: 0, width: innerWidth, height: innerHeight };
    const box = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
    const visible = (r) => r.width > 0 && r.height > 0 && r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight;
    const rgba = (value) => {
      const canvas = document.createElement("canvas"); canvas.width = canvas.height = 1;
      const context = canvas.getContext("2d");
      context.clearRect(0, 0, 1, 1); context.fillStyle = value; context.fillRect(0, 0, 1, 1);
      return [...context.getImageData(0, 0, 1, 1).data].map((n) => n / 255);
    };
    const typography = (s) => ({ family: s.fontFamily.split(",")[0].replace(/["']/g, "").trim(), weight: Number(s.fontWeight) || 400,
      size: parseFloat(s.fontSize), lineHeight: parseFloat(s.lineHeight) || parseFloat(s.fontSize) * 1.2,
      letterSpacing: parseFloat(s.letterSpacing) || 0, color: rgba(s.color), italic: s.fontStyle === "italic",
      decoration: s.textDecorationLine.includes("line-through") ? "STRIKETHROUGH" : s.textDecorationLine.includes("underline") ? "UNDERLINE" : "NONE" });
    function textRuns(node, s, path) {
      // Browser Range supplies the actual line breaks, including mixed inline markup.
      const range = document.createRange(); const runs = [];
      let current;
      let offset = 0;
      for (const char of node.textContent) {
        range.setStart(node, offset); offset += char.length; range.setEnd(node, offset);
        const r = range.getBoundingClientRect();
        if (!visible(r) || !r.width) continue;
        if (current && Math.abs(current.y - r.y) < 0.5 && Math.abs(current.x + current.width - r.x) < 1) {
          current.text += char; current.width = r.right - current.x;
        } else {
          current = { kind: "text", key: `${path}/line-${runs.length}`, name: "Text", ...box(r), text: char, typography: typography(s) };
          runs.push(current);
        }
      }
      return runs;
    }
    async function walk(el, path) {
      if (++count > 6000) throw new Error("Capture exceeds 6000 elements; choose a smaller viewport");
      const s = getComputedStyle(el); const r = el.getBoundingClientRect();
      if (["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "IFRAME"].includes(el.tagName) || s.display === "none" || s.visibility === "hidden" || Number(s.opacity) === 0 || !visible(r)) return null;
      if (el.matches('input[type="password"], [data-genie-capture="exclude"]')) return null;
      const name = `${el.tagName.toLowerCase()}${el.classList.length ? "." + [...el.classList].slice(0, 3).join(".") : ""}`;
      const result = { kind: "frame", key: path, name, ...box(r), opacity: Number(s.opacity),
        fill: rgba(s.backgroundColor), border: ["Top", "Right", "Bottom", "Left"].map((side) => ({ width: parseFloat(s[`border${side}Width`]) || 0, color: rgba(s[`border${side}Color`]) })),
        radius: [s.borderTopLeftRadius, s.borderTopRightRadius, s.borderBottomRightRadius, s.borderBottomLeftRadius].map((v) => parseFloat(v) || 0),
        clip: [s.overflowX, s.overflowY].some((v) => ["hidden", "clip", "auto", "scroll"].includes(v)), children: [] };
      const icon = (key, svg, x, y, width, height) => result.children.push({ kind: "svg", key: `${path}/${key}`, name: key, x, y, width, height,
        svg: `<svg width="${width}" height="${height}" viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg">${svg}</svg>` });
      if (s.transform !== "none") warnings.add("CSS transforms use their axis-aligned browser bounds.");
      if (s.backgroundImage !== "none" || s.boxShadow !== "none" || s.filter !== "none" || s.backdropFilter !== "none") warnings.add("Background images/gradients, shadows and filters are not reproduced.");
      for (const pseudo of ["::before", "::after"]) {
        const p = getComputedStyle(el, pseudo);
        if (p.content && !["none", "normal", '""'].includes(p.content)) warnings.add("Generated pseudo-element content is not reproduced.");
      }
      if (el instanceof SVGElement) {
        const clone = el.cloneNode(true); const original = [el, ...el.querySelectorAll("*")];
        for (const [i, child] of [clone, ...clone.querySelectorAll("*")].entries()) {
          const style = getComputedStyle(original[i]);
          for (const field of ["fill", "stroke", "stroke-width", "opacity", "stroke-linecap", "stroke-linejoin"]) child.setAttribute(field, style.getPropertyValue(field));
          for (const attr of [...child.attributes]) if (attr.name.startsWith("on") || /^(href|xlink:href)$/.test(attr.name)) child.removeAttribute(attr.name);
        }
        clone.setAttribute("xmlns", "http://www.w3.org/2000/svg"); clone.setAttribute("width", String(r.width)); clone.setAttribute("height", String(r.height));
        return { kind: "svg", key: path, name, ...box(r), svg: new XMLSerializer().serializeToString(clone) };
      }
      if (el instanceof HTMLImageElement || el instanceof HTMLCanvasElement) {
        warnings.add("Raster content is omitted; this capture contains no flattened UI images."); return null;
      }
      if (el.matches("button, .btn, .icon-btn, .nav-item, select, input:not([type=checkbox]):not([type=radio])")) result.component = name;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
        warnings.add("Native form chrome is approximated as editable fills, borders and vector marks.");
        let text = el instanceof HTMLSelectElement ? el.selectedOptions[0]?.textContent : el.value || el.placeholder;
        if (el instanceof HTMLInputElement && ["checkbox", "radio"].includes(el.type)) {
          result.radius = [2, 2, 2, 2];
          result.fill = el.checked ? rgba(s.accentColor === "auto" ? s.color : s.accentColor) : rgba("#3b3b3b");
          result.border = Array.from({ length: 4 }, () => ({ width: el.checked ? 0 : 1, color: rgba("#858585") }));
          if (el.checked) icon("checked", '<path d="m3 8 3 3 7-7" fill="none" stroke="white" stroke-width="1.5"/>', r.x, r.y, r.width, r.height);
        } else if (text) {
          const pad = parseFloat(s.paddingLeft) + parseFloat(s.borderLeftWidth);
          result.children.push({ kind: "text", key: path + "/value", name: "Value", x: r.x + pad, y: r.y + (r.height - typography(s).lineHeight) / 2,
            width: Math.max(1, r.width - pad - parseFloat(s.paddingRight)), height: typography(s).lineHeight, text, typography: typography(s) });
          if (!(el instanceof HTMLSelectElement) && !el.value) result.children.at(-1).typography.color = rgba(getComputedStyle(el, "::placeholder").color);
          if (el instanceof HTMLSelectElement) icon("select-arrow", `<path d="m4 6 4 4 4-4" fill="none" stroke="${s.color}" stroke-width="2"/>`, r.right - 18, r.y + (r.height - 12) / 2, 12, 12);
        }
      } else {
        if (el.tagName === "SUMMARY" && s.display === "list-item") icon("disclosure", `<path d="${el.parentElement.open ? "m4 6 4 4 4-4Z" : "m6 3 5 5-5 5Z"}" fill="${s.color}"/>`, r.x, r.y + (r.height - 10) / 2, 10, 10);
        for (const [i, child] of [...el.childNodes].entries()) {
          if (el.tagName === "DETAILS" && !el.open && child.nodeName !== "SUMMARY") continue;
          if (child.nodeType === 3) result.children.push(...textRuns(child, s, `${path}/${i}`));
          else if (child.nodeType === 1) { const captured = await walk(child, `${path}/${i}`); if (captured) result.children.push(captured); }
        }
      }
      return result;
    }
    const root = await walk(document.body, "body");
    if (!root) throw new Error("The page has no visible body");
    root.x = root.y = 0; root.width = innerWidth; root.height = innerHeight; root.clip = true;
    root.fill = rgba(getComputedStyle(document.body).backgroundColor);
    const theme = document.documentElement.dataset.theme || "dark";
    const accent = document.documentElement.dataset.accent || "violet";
    return { format: "genie-screen", version, name: `${location.pathname} · ${innerWidth}×${innerHeight}`, mode: `${theme}-${accent}`,
      viewport, root, warnings: [...warnings], capturedAt: new Date().toISOString() };
  }
  async function download() {
    const data = await capture(); const blob = new Blob([JSON.stringify(data)], { type: "application/json" });
    const url = URL.createObjectURL(blob); const link = document.createElement("a");
    link.href = url; link.download = "genie-screen.json"; link.click(); URL.revokeObjectURL(url);
    return { name: data.name, warnings: data.warnings };
  }
  return { capture, download };
})();
