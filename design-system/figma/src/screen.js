/* Fixed-viewport DOM snapshots: native editable layers, never a full-screen bitmap. */
const GenieScreen = (() => {
  function validate(data) {
    if (!data || data.format !== "genie-screen" || data.version !== 1 || typeof data.name !== "string" || data.name.length > 200
      || !/^(dark|light)-(violet|blue|teal|green|orange|rose)$/.test(data.mode)
      || (data.warnings !== undefined && (!Array.isArray(data.warnings) || data.warnings.length > 100 || data.warnings.some((w) => typeof w !== "string")))) throw new Error("Expected a Genie screen capture v1");
    let count = 0;
    const keys = new Set();
    const finite = (v, min, max) => typeof v === "number" && Number.isFinite(v) && v >= min && v <= max;
    const color = (v) => Array.isArray(v) && v.length === 4 && v.every((n) => finite(n, 0, 1));
    function walk(n, depth) {
      if (++count > 12000 || depth > 100) throw new Error("Screen capture is too large or deeply nested");
      if (!n || !["frame", "text", "svg"].includes(n.kind) || typeof n.key !== "string" || keys.has(n.key)
        || typeof n.name !== "string" || !finite(n.x, -100000, 100000) || !finite(n.y, -100000, 100000)
        || !finite(n.width, 0.001, 100000) || !finite(n.height, 0.001, 100000)) throw new Error("Invalid screen node geometry or duplicate key");
      keys.add(n.key);
      if (n.kind === "frame") {
        if (!color(n.fill) || !finite(n.opacity, 0, 1) || !Array.isArray(n.radius) || n.radius.length !== 4 || !n.radius.every((v) => finite(v, 0, 100000))
          || !Array.isArray(n.border) || n.border.length !== 4 || !n.border.every((b) => finite(b.width, 0, 1000) && color(b.color)) || !Array.isArray(n.children)) throw new Error("Invalid frame paint");
        for (const child of n.children) walk(child, depth + 1);
      } else if (n.kind === "text") {
        const t = n.typography;
        if (typeof n.text !== "string" || n.text.length > 50000 || !t || typeof t.family !== "string" || !color(t.color)
          || !finite(t.weight, 100, 900) || !finite(t.size, 1, 1000) || !finite(t.lineHeight, 1, 2000) || !finite(t.letterSpacing, -100, 100)) throw new Error("Invalid text style");
      } else if (typeof n.svg !== "string" || n.svg.length > 200000 || !/^<svg[\s>]/.test(n.svg)
        || /<(script|image|foreignObject|use)\b|\bon\w+\s*=|(?:href|url)\s*[=(]/i.test(n.svg)) throw new Error("Unsupported SVG content");
    }
    walk(data.root, 0);
    if (data.root.kind !== "frame" || data.root.width > 10000 || data.root.height > 10000) throw new Error("Invalid screen viewport");
    return count;
  }

  async function build(data, allowFallback) {
    validate(data);
    const available = await figma.listAvailableFontsAsync();
    const fonts = new Map(); const sources = [];
    const walk = (n) => { sources.push(n); for (const c of n.children || []) walk(c); }; walk(data.root);
    const fontKey = (t) => `${t.family}/${t.weight}/${!!t.italic}`;
    const normalize = (s) => s.replace(/\s/g, "").toLowerCase();
    for (const n of sources.filter((n) => n.kind === "text")) {
      const t = n.typography;
      const weight = { 100: "Thin", 200: "ExtraLight", 300: "Light", 400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold", 900: "Black" }[Math.round(t.weight / 100) * 100];
      const style = t.italic && weight === "Regular" ? "Italic" : weight + (t.italic ? "Italic" : "");
      const matches = (f) => normalize(f.fontName.style) === normalize(style);
      const font = available.find((f) => f.fontName.family === t.family && matches(f))
        || (allowFallback && available.find((f) => f.fontName.family === "Inter" && matches(f)));
      if (!font) throw new Error(`Missing font ${t.family} ${style}; install it or explicitly allow Inter fallback`);
      fonts.set(fontKey(t), font.fontName);
    }
    await Promise.all([...fonts.values()].map((f) => figma.loadFontAsync(f)));
    const collections = await figma.variables.getLocalVariableCollectionsAsync();
    const collection = collections.find((c) => c.name === `Genie / ${data.mode}`);
    const vars = (await figma.variables.getLocalVariablesAsync()).filter((v) => v.variableCollectionId === collection?.id);
    const styles = (await figma.getLocalTextStylesAsync()).filter((s) => s.name.startsWith(`Genie / ${data.mode}/`));
    function resolved(v, seen = new Set()) {
      if (seen.has(v.id)) return null; seen.add(v.id);
      const value = v.valuesByMode?.[collection.defaultModeId];
      return value?.type === "VARIABLE_ALIAS" ? resolved(vars.find((a) => a.id === value.id) || { id: "missing" }, seen) : value;
    }
    const colors = vars.filter((v) => v.resolvedType === "COLOR").map((v) => ({ variable: v, value: resolved(v) }));
    let bound = 0;
    function paints(c) {
      if (!c[3]) return [];
      const p = { type: "SOLID", color: { r: c[0], g: c[1], b: c[2] }, opacity: c[3] };
      const match = colors.find(({ value: v }) => v && Math.abs(v.r - c[0]) < 1 / 255 && Math.abs(v.g - c[1]) < 1 / 255 && Math.abs(v.b - c[2]) < 1 / 255 && Math.abs((v.a ?? 1) - c[3]) < 1 / 255);
      if (match) { bound++; return [figma.variables.setBoundVariableForPaint(p, "color", match.variable)]; }
      return [p];
    }
    const name = `Genie / ${data.mode} / Capture ${data.name}`;
    // Each snapshot is append-only; an earlier import may contain independent user edits.
    const root = figma.createFrame(); root.name = name;
    figma.currentPage.appendChild(root);
    const others = figma.currentPage.children.filter((n) => n.id !== root.id);
    root.x = Math.max(120, ...others.map((n) => n.x + n.width + 240)); root.y = 120;
    const created = [root.id]; const mains = new Map(); let instances = 0;
    let components;
    function signature(n, origin = n) {
      return JSON.stringify({ ...n, key: undefined, x: n.x - origin.x, y: n.y - origin.y,
        children: (n.children || []).map((c) => signature(c, n)) });
    }
    async function build(n, parent, origin, isRoot = false) {
      if (n.component && !isRoot) {
        const key = signature(n);
        let main = mains.get(key);
        if (!main) {
          if (!components) {
            components = figma.createFrame(); components.name = `${name} / Source components`; components.fills = [];
            figma.currentPage.appendChild(components); components.x = root.x; components.y = root.y + data.root.height + 240;
          }
          main = figma.createComponent(); main.name = n.component; components.appendChild(main);
          await fill(main, n); main.x = (mains.size % 4) * 340; main.y = Math.floor(mains.size / 4) * 120;
          for (const c of n.children) await build(c, main, n);
          for (const [i, text] of main.findAllWithCriteria({ types: ["TEXT"] }).entries()) {
            const property = main.addComponentProperty(`Text ${i + 1}`, "TEXT", text.characters);
            text.componentPropertyReferences = { ...text.componentPropertyReferences, characters: property };
          }
          mains.set(key, main); created.push(main.id);
        }
        const instance = main.createInstance(); parent.appendChild(instance); instance.name = n.name;
        instance.x = n.x - origin.x; instance.y = n.y - origin.y; created.push(instance.id); instances++;
        return instance;
      }
      let node = isRoot ? root : n.kind === "text" ? figma.createText() : n.kind === "svg" ? figma.createNodeFromSvg(n.svg) : figma.createFrame();
      if (!isRoot) parent.appendChild(node);
      created.push(node.id); if (!isRoot) node.name = n.name;
      if (!isRoot) { node.x = n.x - origin.x; node.y = n.y - origin.y; }
      if (n.kind === "text") {
        const t = n.typography; node.fontName = fonts.get(fontKey(t));
        const style = styles.find((s) => s.fontName.family === node.fontName.family && normalize(s.fontName.style) === normalize(node.fontName.style) && Math.abs(s.fontSize - t.size) < 0.01);
        if (style) await node.setTextStyleIdAsync(style.id);
        node.fontSize = t.size; node.lineHeight = { unit: "PIXELS", value: t.lineHeight }; node.letterSpacing = { unit: "PIXELS", value: t.letterSpacing };
        node.characters = n.text; node.fills = paints(t.color);
        // Each source run is already one browser line. Native glyph metrics differ
        // slightly; fixed widths can wrap a second time and overlap the next line.
        node.textAutoResize = "WIDTH_AND_HEIGHT";
        node.y -= (t.lineHeight - n.height) / 2;
        node.textDecoration = t.decoration || "NONE";
      } else if (n.kind === "svg") node.resize(n.width, n.height);
      else { await fill(node, n); for (const child of n.children) await build(child, node, n); }
      return node;
    }
    async function fill(node, n) {
      node.layoutMode = "NONE"; node.resize(n.width, n.height); node.fills = paints(n.fill); node.opacity = n.opacity; node.clipsContent = n.clip;
      [node.topLeftRadius, node.topRightRadius, node.bottomRightRadius, node.bottomLeftRadius] = n.radius;
      // Figma supports individual widths, with one shared stroke paint.
      const stroke = n.border.find((b) => b.width && b.color[3]);
      node.strokes = stroke ? paints(stroke.color) : []; node.strokeAlign = "INSIDE";
      [node.strokeTopWeight, node.strokeRightWeight, node.strokeBottomWeight, node.strokeLeftWeight] = n.border.map((b) => b.width);
    }
    try {
      await build(data.root, figma.currentPage, { x: 0, y: 0 }, true);
      if (components) {
        const column = Math.max(340, ...[...mains.values()].map((n) => n.width + 24));
        const row = Math.max(120, ...[...mains.values()].map((n) => n.height + 24));
        for (const [i, main] of [...mains.values()].entries()) { main.x = (i % 4) * column; main.y = Math.floor(i / 4) * row; }
        components.resize(column * 4, Math.max(row, Math.ceil(mains.size / 4) * row));
      }
      figma.currentPage.selection = [root]; figma.viewport.scrollAndZoomIntoView([root]);
      return { screenId: root.id, componentFrameId: components?.id, nodes: sources.length, componentInstances: instances,
        sourceComponents: mains.size, boundPaints: bound, fonts: [...new Set([...fonts.values()].map((f) => f.family))], ids: [...new Set(created)],
        warnings: [...(data.warnings || []), ...(!collection ? ["No matching Genie variable collection; import this appearance's tokens to enable bindings on the next screen import."] : [])] };
    } catch (error) {
      root.remove(); if (components) components.remove(); throw error;
    }
  }
  return { validate, build };
})();
