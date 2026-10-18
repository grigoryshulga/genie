// A strict behavioral fake for the native APIs used by our offline plugin.
// It does not simulate Figma's renderer; visual verification is separate.
export function createFigmaPluginHost() {
  let sequence = 0;
  const nodes = new Map();
  const loadedFonts = new Set();
  const textStyles = [];
  const variables = [];
  const collections = [];
  const effectStyles = [];
  const messages = [];
  const fontKey = (font) => `${font.family}/${font.style}`;

  class Node {
    constructor(type) {
      this.id = String(++sequence);
      this.type = type;
      this.name = type;
      /** @type {Node[]} */
      this.children = [];
      this.parent = null;
      this.x = this.y = 0;
      this._width = this._height = 100;
      this.primaryAxisSizingMode = this.counterAxisSizingMode = "FIXED";
      this.fontName = { family: "Inter", style: "Regular" };
      this.fills = [];
      /** @type {Record<string, {type: string, id: string}>} */
      this.boundVariables = {};
      /** @type {Record<string, string>} */
      this.componentPropertyReferences = {};
      this.definitions = {};
      this.instanceValues = {};
      this.content = "";
      nodes.set(this.id, this);
    }
    appendChild(child) {
      if (child.parent) child.parent.children = child.parent.children.filter((n) => n !== child);
      child.parent = this;
      this.children.push(child);
    }
    remove() {
      for (const child of [...this.children]) child.remove();
      if (this.parent) this.parent.children = this.parent.children.filter((n) => n !== this);
      nodes.delete(this.id);
    }
    get width() { return this._width; }
    get height() { return this._height; }
    resize(width, height) {
      this._width = width; this._height = height;
      this.primaryAxisSizingMode = this.counterAxisSizingMode = "FIXED";
    }
    setBoundVariable(field, variable) {
      if (!variable) throw new Error(`Missing variable for ${field}`);
      this.boundVariables[field] = { type: "VARIABLE_ALIAS", id: variable.id };
    }
    get characters() { return this.content; }
    set characters(value) {
      if (this.type !== "TEXT") throw new Error("Not a text node");
      if (!loadedFonts.has(fontKey(this.fontName))) throw new Error(`Font not loaded: ${fontKey(this.fontName)}`);
      this.content = value;
    }
    getStyledTextSegments() { return [{ fontName: this.fontName }]; }
    async setTextStyleIdAsync(id) {
      const style = textStyles.find((item) => item.id === id);
      if (!style) throw new Error(`Missing text style ${id}`);
      this.textStyleId = id;
      this.fontName = style.fontName;
      this.fontSize = style.fontSize;
    }
    /** @returns {Node[]} */
    findAllWithCriteria({ types }) {
      return this.children.flatMap((child) => [...(types.includes(child.type) ? [child] : []), ...child.findAllWithCriteria({ types })]);
    }
    get componentPropertyDefinitions() {
      if (this.type === "COMPONENT" && this.parent?.type === "COMPONENT_SET") throw new Error("Cannot read property definitions on a variant");
      if (this.type === "COMPONENT_SET") return Object.assign({}, ...this.children.map((c) => c.definitions), this.definitions);
      return this.definitions;
    }
    addComponentProperty(name, type, defaultValue) {
      const key = `${name}#${++sequence}`;
      this.definitions[key] = { type, defaultValue };
      return key;
    }
    createInstance() {
      if (this.type !== "COMPONENT") throw new Error("Expected component");
      const clone = (source) => {
        const target = new Node(source.type === "COMPONENT" ? "INSTANCE" : source.type);
        target.resize(source.width, source.height);
        for (const key of ["fontName", "fontSize", "textStyleId", "name", "content", "fills", "boundVariables", "componentPropertyReferences"]) target[key] = structuredClone(source[key]);
        for (const child of source.children) target.appendChild(clone(child));
        return target;
      };
      const target = clone(this);
      target.main = this;
      return target;
    }
    async getMainComponentAsync() { return this.main; }
    get componentProperties() {
      if (this.type !== "INSTANCE") throw new Error("Expected instance");
      const owner = this.main.parent?.type === "COMPONENT_SET" ? this.main.parent : this.main;
      return Object.fromEntries(Object.entries(owner.componentPropertyDefinitions).map(([key, definition]) => [key, { type: definition.type, value: this.instanceValues[key] ?? definition.defaultValue }]));
    }
    setProperties(props) {
      for (const key of Object.keys(props)) if (!this.componentProperties[key]) throw new Error(`Unknown instance property ${key}`);
      Object.assign(this.instanceValues, props);
    }
  }
  const page = new Node("PAGE");
  page.name = "User page";
  const figma = {
    editorType: "figma",
    currentPage: page,
    viewport: { scrollAndZoomIntoView: () => {} },
    availableFonts: ["Geist", "Geist Mono", "Inter"].flatMap((family) => ["Regular", "Medium", "Semi Bold"].map((style) => ({ fontName: { family, style } }))),
    listAvailableFontsAsync: async () => figma.availableFonts,
    loadFontAsync: async (font) => {
      if (!figma.availableFonts.some((f) => fontKey(f.fontName) === fontKey(font))) throw new Error("Unavailable font");
      loadedFonts.add(fontKey(font));
    },
    showUI: () => {},
    ui: { postMessage: (message) => messages.push(message), onmessage: async (_message) => {} },
    createFrame: () => new Node("FRAME"),
    createComponent: () => new Node("COMPONENT"),
    createText: () => new Node("TEXT"),
    createNodeFromSvg: (_svg) => new Node("FRAME"),
    combineAsVariants: (components, parent) => {
      const set = new Node("COMPONENT_SET"); parent.appendChild(set);
      for (const component of components) set.appendChild(component);
      return set;
    },
    getLocalTextStylesAsync: async () => textStyles,
    createTextStyle: () => {
      const style = { id: String(++sequence), boundVariables: {}, setBoundVariable(field, variable) { this.boundVariables[field] = { type: "VARIABLE_ALIAS", id: variable.id }; } };
      textStyles.push(style); return style;
    },
    getLocalEffectStylesAsync: async () => effectStyles,
    createEffectStyle: () => { const style = { id: String(++sequence) }; effectStyles.push(style); return style; },
    variables: {
      getLocalVariableCollectionsAsync: async () => collections,
      getLocalVariablesAsync: async () => variables,
      setBoundVariableForPaint: (paint, _field, variable) => {
        if (!variable || variable.resolvedType !== "COLOR") throw new Error("Expected color variable");
        return { ...paint, boundVariables: { color: { type: "VARIABLE_ALIAS", id: variable.id } } };
      },
      createVariableCollection: (name) => {
        const collection = { id: String(++sequence), name, defaultModeId: "default" };
        collections.push(collection); return collection;
      },
      createVariable: (name, collection, resolvedType) => {
        const variable = {
          id: String(++sequence), name, resolvedType, variableCollectionId: collection.id, values: {}, valuesByMode: {},
          setValueForMode(mode, value) { this.values[mode] = value; this.valuesByMode[mode] = value; },
          setVariableCodeSyntax(_platform, value) { this.syntax = value; },
        };
        variables.push(variable); return variable;
      },
    },
  };
  return { figma, page, nodes, textStyles, variables, collections, effectStyles, messages, loadedFonts };
}
