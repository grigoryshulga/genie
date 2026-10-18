/* Offline Figma development plugin. Input is the generated ../tokens.json. */
figma.showUI(__html__, { width: 420, height: 480, themeColors: true });

function variableType(token) {
  if (token.$type === "color") return "COLOR";
  if (token.$type === "dimension" || token.$type === "number") return "FLOAT";
  if (token.$type === "fontFamily") return "STRING";
  if (token.$type === "shadow") return null;
  throw new Error(`Unsupported token type: ${token.$type}`);
}

function color(value) {
  if (value.colorSpace !== "srgb" || value.components.length !== 3
    || ![...value.components, value.alpha].every((n) => Number.isFinite(n) && n >= 0 && n <= 1)) throw new Error("Invalid sRGB color");
  return { r: value.components[0], g: value.components[1], b: value.components[2], a: value.alpha };
}

function value(token) {
  if (token.$type === "color") return color(token.$value);
  if (token.$type === "dimension") {
    if (token.$value.unit !== "px" || !Number.isFinite(token.$value.value)) throw new Error("Expected a pixel dimension");
    return token.$value.value;
  }
  if (token.$type === "fontFamily") return token.$value[0];
  if (token.$type === "number" && Number.isFinite(token.$value)) return token.$value;
  throw new Error("Invalid token value");
}

function scopes(key, type) {
  if (type === "COLOR") {
    if (/^(line|accent-line|.*-line)/.test(key)) return ["STROKE_COLOR"];
    if (/^(text|muted|on-|role-text|accent-text)/.test(key)) return ["TEXT_FILL"];
    return ["FRAME_FILL", "SHAPE_FILL", "TEXT_FILL", "STROKE_COLOR"];
  }
  if (key.startsWith("radius")) return ["CORNER_RADIUS"];
  if (key.startsWith("space-")) return ["GAP", "WIDTH_HEIGHT"];
  if (key.startsWith("font-size-")) return ["FONT_SIZE"];
  if (key.startsWith("weight-")) return ["FONT_WEIGHT"];
  if (key.startsWith("control-") || key === "content") return ["WIDTH_HEIGHT"];
  return [];
}

let importing = false;
figma.ui.onmessage = async (message) => {
  if (!["import", "import-screen"].includes(message.type) || importing) return;
  importing = true;
  try {
    if (message.type === "import-screen") {
      const screen = await GenieScreen.build(message.screen, message.allowFallback === true);
      figma.ui.postMessage({ ok: true, screen, text: `Imported ${screen.nodes} editable source nodes and ${screen.componentInstances} component instances. Fonts: ${screen.fonts.join(", ")}. ${screen.warnings.join(" ")}` });
      return;
    }
    if (!/^(dark|light)-(violet|blue|teal|green|orange|rose)$/.test(message.mode)) throw new Error("Choose an exported theme/accent pair");
    const entries = Object.entries(message.tokens).filter(([key]) => !key.startsWith("$"));
    if (!entries.length) throw new Error("No tokens found");
    const fonts = message.buildLibrary ? await GenieLibrary.preflight(message.tokens, message.mode, message.allowFallback === true) : null;
    const name = `Genie / ${message.mode}`;
    const collections = await figma.variables.getLocalVariableCollectionsAsync();
    let collection = collections.find((item) => item.name === name);
    const variables = await figma.variables.getLocalVariablesAsync();
    const existing = new Map(variables.filter((item) => item.variableCollectionId === (collection && collection.id)).map((item) => [item.name, item]));
    // Validate the entire input and existing variable types before mutating the document.
    for (const [key, token] of entries) {
      if (!/^[\w-]+$/.test(key) || token.$extensions["com.genie"].css !== `var(--${key})`) throw new Error(`Invalid CSS mapping: ${key}`);
      const type = variableType(token);
      if (type) {
        value(token);
        const previous = existing.get(key.replace(/-/g, "/"));
        if (previous && previous.resolvedType !== type) throw new Error(`Type changed for ${key}; resolve this conflict in Figma first`);
      } else {
        color(token.$value.color);
        for (const field of ["offsetX", "offsetY", "blur", "spread"]) {
          if (token.$value[field].unit !== "px" || !Number.isFinite(token.$value[field].value)) throw new Error(`Invalid shadow ${field}`);
        }
      }
    }
    if (!collection) collection = figma.variables.createVariableCollection(name);
    const modeId = collection.defaultModeId;
    const imported = new Map();
    for (const [key, token] of entries) {
      const type = variableType(token);
      if (!type) continue;
      const variableName = key.replace(/-/g, "/");
      const variable = existing.get(variableName) || figma.variables.createVariable(variableName, collection, type);
      variable.scopes = scopes(key, type);
      variable.description = `Source: --${key}. ${token.$extensions["com.genie"].expression}`;
      variable.setVariableCodeSyntax("WEB", `var(--${key})`);
      variable.setValueForMode(modeId, value(token));
      imported.set(key, variable);
    }
    // Keep direct CSS references as native variable aliases within the selected collection.
    for (const [key, token] of entries) {
      const reference = /^var\(--([\w-]+)\)$/.exec(token.$extensions["com.genie"].expression);
      if (reference && imported.has(reference[1])) imported.get(key).setValueForMode(modeId, { type: "VARIABLE_ALIAS", id: imported.get(reference[1]).id });
    }
    const styles = await figma.getLocalEffectStylesAsync();
    for (const [key, token] of entries) {
      if (token.$type !== "shadow") continue;
      const styleName = `${name}/${key}`;
      const style = styles.find((item) => item.name === styleName) || figma.createEffectStyle();
      style.name = styleName;
      style.description = `Source: var(--${key})`;
      const shadow = token.$value;
      style.effects = [{ type: "DROP_SHADOW", color: color(shadow.color), offset: { x: shadow.offsetX.value, y: shadow.offsetY.value }, radius: shadow.blur.value, spread: shadow.spread.value, visible: true, blendMode: "NORMAL" }];
    }
    const design = fonts ? await GenieLibrary.build({ mode: message.mode, tokens: message.tokens, variables: imported, fonts }) : null;
    figma.ui.postMessage({ ok: true, design, text: design
      ? `Built ${design.textStyles} text styles, ${design.componentVariants} component variants and a task screen. Fonts: ${design.fonts.join(", ")}. Reimport updates generated content; matching IDs and unrelated layers are preserved.`
      : `Imported ${imported.size} variables and overlay shadow into ${name}. Existing IDs were preserved.` });
  } catch (error) {
    figma.ui.postMessage({ ok: false, text: String(error) });
  } finally {
    importing = false;
  }
};
