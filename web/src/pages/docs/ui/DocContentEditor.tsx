import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { EditorContent, useEditor, useEditorState } from "@tiptap/react";
import { Markdown } from "@tiptap/markdown";
import { DocBody } from "@/entities/doc";
import { Icon } from "@/shared/ui";
import { articleExtensions, visualEditingIssue } from "../model/rich-markdown.ts";
import { formatMarkdown, type ArticleFormat } from "../model/format-markdown.ts";

type Mode = "visual" | "source" | "split";
const MODES: { id: Mode; label: string; icon: ReactNode }[] = [
  { id: "visual", label: "Статья", icon: <Icon.file size={14} /> }, { id: "source", label: "Markdown", icon: <Icon.markdown size={14} /> }, { id: "split", label: "Вместе", icon: <Icon.columns size={14} /> },
];
const FORMATS: { id: ArticleFormat; label: string; icon: string }[] = [
  { id: "bold", label: "Жирный (Ctrl+B)", icon: "B" }, { id: "italic", label: "Курсив (Ctrl+I)", icon: "I" },
  { id: "strike", label: "Зачёркнутый", icon: "S̶" }, { id: "h1", label: "Заголовок 1", icon: "H1" },
  { id: "h2", label: "Заголовок 2", icon: "H2" }, { id: "h3", label: "Заголовок 3", icon: "H3" },
  { id: "bullet", label: "Маркированный список", icon: "• ≡" }, { id: "ordered", label: "Нумерованный список", icon: "1. ≡" },
  { id: "quote", label: "Цитата", icon: "❞" }, { id: "code", label: "Код в строке", icon: "<>" },
  { id: "codeBlock", label: "Блок кода", icon: "{ }" }, { id: "divider", label: "Разделитель", icon: "―" },
  { id: "link", label: "Ссылка", icon: "↗" },
];

export function DocContentEditor({ value, onChange, title }: { value: string; onChange: (value: string) => void; title: string }) {
  const [mode, setMode] = useState<Mode>("visual");
  const [link, setLink] = useState<string | null>(null);
  const source = useRef<HTMLTextAreaElement>(null);
  const selection = useRef({ start: 0, end: 0 });
  const lastOutput = useRef(value);
  const issue = useMemo(() => visualEditingIssue(value), [value]);
  const effectiveMode = mode === "visual" && issue ? "source" : mode;
  const extensions = useMemo(() => [...articleExtensions(), Markdown], []);
  const editor = useEditor({
    extensions, content: value, contentType: "markdown",
    // The same prose typography as the article reader (g-prose), so the editor shows what the page will look like.
    editorProps: { attributes: { class: "g-prose", role: "textbox", "aria-label": "Текст статьи", "aria-multiline": "true" } },
    onUpdate: ({ editor }) => {
      const markdown = editor.getMarkdown();
      lastOutput.current = markdown;
      onChange(markdown);
    },
  });
  const state = useEditorState({ editor, selector: ({ editor }) => editor ? {
    bold: editor.isActive("bold"), italic: editor.isActive("italic"), strike: editor.isActive("strike"), code: editor.isActive("code"),
    h1: editor.isActive("heading", { level: 1 }), h2: editor.isActive("heading", { level: 2 }), h3: editor.isActive("heading", { level: 3 }),
    bullet: editor.isActive("bulletList"), ordered: editor.isActive("orderedList"), quote: editor.isActive("blockquote"), codeBlock: editor.isActive("codeBlock"),
    link: editor.isActive("link"), undo: editor.can().undo(), redo: editor.can().redo(),
  } : null });

  useEffect(() => {
    if (editor && value !== lastOutput.current && !issue) {
      editor.commands.setContent(value, { contentType: "markdown", emitUpdate: false });
      lastOutput.current = value;
    }
  }, [editor, value, issue]);

  const apply = (format: ArticleFormat, url?: string) => {
    if (effectiveMode !== "visual") {
      const area = source.current;
      if (!area) return;
      const edit = formatMarkdown(value, selection.current.start, selection.current.end, format, url);
      onChange(edit.text);
      requestAnimationFrame(() => { area.focus(); area.setSelectionRange(edit.start, edit.end); selection.current = edit; });
      return;
    }
    if (!editor) return;
    const chain = editor.chain().focus();
    switch (format) {
      case "bold": chain.toggleBold().run(); break;
      case "italic": chain.toggleItalic().run(); break;
      case "strike": chain.toggleStrike().run(); break;
      case "code": chain.toggleCode().run(); break;
      case "h1": chain.toggleHeading({ level: 1 }).run(); break;
      case "h2": chain.toggleHeading({ level: 2 }).run(); break;
      case "h3": chain.toggleHeading({ level: 3 }).run(); break;
      case "bullet": chain.toggleBulletList().run(); break;
      case "ordered": chain.toggleOrderedList().run(); break;
      case "quote": chain.toggleBlockquote().run(); break;
      case "codeBlock": chain.toggleCodeBlock().run(); break;
      case "divider": chain.setHorizontalRule().run(); break;
      case "link": url ? chain.extendMarkRange("link").setLink({ href: url }).run() : chain.extendMarkRange("link").unsetLink().run(); break;
    }
  };
  const rememberSelection = () => {
    if (source.current) selection.current = { start: source.current.selectionStart, end: source.current.selectionEnd };
  };
  const shortcut = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!(event.metaKey || event.ctrlKey)) return;
    const command = event.key.toLowerCase() === "b" ? "bold" : event.key.toLowerCase() === "i" ? "italic" : undefined;
    if (command) { event.preventDefault(); rememberSelection(); apply(command); }
  };

  return <div className="doc-content-editor">
    <div className="doc-format-toolbar" role="toolbar" aria-label="Форматирование статьи">
      {FORMATS.map((format) => <button key={format.id} type="button" aria-label={format.label} title={format.label} aria-pressed={effectiveMode === "visual" && format.id !== "divider" ? !!state?.[format.id] : undefined} onMouseDown={(e) => e.preventDefault()} onClick={() => format.id === "link" ? setLink(effectiveMode === "visual" ? String(editor?.getAttributes("link").href ?? "https://") : "https://") : apply(format.id)}>{format.icon}</button>)}
      {effectiveMode === "visual" && <>
        <button type="button" aria-label="Отменить" title="Отменить (Ctrl+Z)" disabled={!state?.undo} onMouseDown={(e) => e.preventDefault()} onClick={() => editor?.chain().focus().undo().run()}>↶</button>
        <button type="button" aria-label="Повторить" title="Повторить (Ctrl+Shift+Z)" disabled={!state?.redo} onMouseDown={(e) => e.preventDefault()} onClick={() => editor?.chain().focus().redo().run()}>↷</button>
      </>}
      {issue && <span className="doc-editor-format-hint">{issue}</span>}
      <div className="g-seg doc-editor-modes" role="group" aria-label="Режим редактора">
        {MODES.map((item) => <button type="button" key={item.id} aria-label={item.label} aria-pressed={effectiveMode === item.id} disabled={item.id === "visual" && !!issue} title={item.id === "visual" && issue ? issue : item.label} className={effectiveMode === item.id ? "on" : ""} onClick={() => { setMode(item.id); setLink(null); }}>{item.icon}</button>)}
      </div>
    </div>
    {link !== null && <form className="doc-link-form" onSubmit={(e) => { e.preventDefault(); apply("link", link); setLink(null); }}>
      <label>Адрес ссылки <input autoFocus aria-label="Адрес ссылки" value={link} onChange={(e) => setLink(e.target.value)} /></label>
      <button type="submit" className="g-btn secondary sm">Применить</button><button type="button" className="g-btn ghost sm" onClick={() => setLink(null)}>Отмена</button>
    </form>}
    {effectiveMode === "visual" ? <div className="doc-rich-editor scroll doc-body"><EditorContent editor={editor} /></div> : <div className={`doc-split doc-content-${effectiveMode}`}>
      <section className="doc-pane">
        <header className="doc-pane-head"><span>Markdown</span><span>Исходный текст</span></header>
        <textarea ref={source} className="doc-textarea" aria-label="Markdown статьи" spellCheck={false} value={value} onChange={(e) => { onChange(e.target.value); rememberSelection(); }} onSelect={rememberSelection} onKeyDown={shortcut} />
      </section>
      {effectiveMode === "split" && <section className="doc-pane">
        <header className="doc-pane-head"><span>Предпросмотр</span><span>{title}</span></header>
        <div className="doc-preview scroll"><DocBody text={value} empty="Пусто" /></div>
      </section>}
    </div>}
  </div>;
}
