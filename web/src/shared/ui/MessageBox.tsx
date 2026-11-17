// The one box every message is written in: the project chat, the chat with an agent, the team's mail, a task's
// comments and the owner's answer. A single field in a frame; the text renders as Markdown while it is typed
// (`**bold**`, `- ` lists, `# ` headings, `` `code` ``) and what goes out is the Markdown text. The frame carries
// the controls: an icon in the corner widens the box over the card or page it sits in, a round arrow in the
// bottom corner sends, and a caller's own controls (who gets it, how urgent) sit at the bottom left.

import { EditorContent, useEditor } from "@tiptap/react";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";
import { type CSSProperties, type ReactNode, type Ref, useEffect, useId, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./icons.tsx";
import { cx } from "./kit/cx.ts";
import "./message-box.css";

export interface MessageBoxHandle {
  focus: () => void;
}

/** One thing the box can insert from a list that opens as the person types (an `@` mention). */
export interface Suggestion {
  key: string;
  label: string;
  /** What replaces the typed token. */
  insert: string;
}

export interface SuggestMatch {
  /** Where the token starts in the text before the caret. */
  start: number;
  items: Suggestion[];
}

export interface MessageBoxProps {
  /** The Markdown text. */
  value: string;
  onChange: (markdown: string) => void;
  onSubmit: () => void;
  /** Names the field for assistive technology. */
  label: string;
  placeholder: string;
  canSend: boolean;
  /** The send arrow's name; default "Отправить". */
  sendLabel?: string;
  sendTitle?: string;
  /** Enter sends and Shift+Enter starts a line ("enter"), or only ⌘/Ctrl+Enter sends ("mod-enter"). ⌘/Ctrl+Enter always sends. */
  submitOn?: "enter" | "mod-enter";
  /** A list that opens above the box: `match` gets the line's text before the caret. */
  suggest?: { label: string; match: (before: string) => SuggestMatch | null };
  /** Inside the frame, above the field (a quote of what is answered). */
  top?: ReactNode;
  /** The bottom left of the frame. */
  footer?: ReactNode;
  /** The field's least height in pixels; default one comfortable line. */
  minHeight?: number;
  /** Escape in the field, when it has nothing of its own to close. */
  onEscape?: () => void;
  onFocus?: () => void;
  /** The look of an urgent message. */
  urgent?: boolean;
  className?: string;
  handle?: Ref<MessageBoxHandle>;
}

/** The part of the page a widened box covers, cut to the window. */
function coverRect(from: HTMLElement | null): CSSProperties | undefined {
  const card = from?.closest<HTMLElement>("[data-box-cover], .detail, .task-page, main");
  if (!card) return undefined;
  const r = card.getBoundingClientRect();
  const top = Math.max(r.top, 0);
  const left = Math.max(r.left, 0);
  return { top, left, width: Math.min(r.right, window.innerWidth) - left, height: Math.min(r.bottom, window.innerHeight) - top };
}

export function MessageBox({
  value,
  onChange,
  onSubmit,
  label,
  placeholder,
  canSend,
  sendLabel = "Отправить",
  sendTitle,
  submitOn = "mod-enter",
  suggest,
  top,
  footer,
  minHeight = 48,
  onEscape,
  onFocus,
  urgent,
  className,
  handle,
}: MessageBoxProps) {
  const root = useRef<HTMLDivElement>(null);
  const last = useRef(value);
  const [wide, setWide] = useState(false);
  const [rect, setRect] = useState<CSSProperties>();
  const [match, setMatch] = useState<SuggestMatch | null>(null);
  const [index, setIndex] = useState(0);
  // Escape closes the list until the next keystroke; moving the caret does not reopen it.
  const [closed, setClosed] = useState(false);
  const listId = useId();
  // The editor is created once; what it calls is read from here.
  const live = useRef({ onSubmit, submitOn, suggest, onEscape, onFocus, match, index, closed, wide, canSend });
  live.current = { onSubmit, submitOn, suggest, onEscape, onFocus, match, index, closed, wide, canSend };
  const extensions = useMemo(() => [StarterKit.configure({ underline: false, link: { openOnClick: false } }), Markdown], []);

  const options = match?.items ?? [];
  const open = options.length > 0 && !closed;
  const active = Math.min(index, Math.max(0, options.length - 1));
  const optionId = (i: number) => `${listId}-${i}`;

  const editor = useEditor({
    extensions,
    content: value,
    contentType: "markdown",
    editorProps: {
      attributes: { class: "mbx-prose", role: "textbox", "aria-label": label, "aria-multiline": "true", "aria-placeholder": placeholder },
      handleKeyDown: (view, e) => {
        const s = live.current;
        if (e.isComposing) return false;
        if (e.key === "Escape" && s.wide) return false;
        const items = s.match?.items ?? [];
        const listOpen = items.length > 0 && !s.closed;
        if (listOpen) {
          const at = Math.min(s.index, items.length - 1);
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            setIndex((at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length);
            return true;
          }
          if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
            pick(items[at]);
            return true;
          }
          if (e.key === "Escape") {
            e.stopPropagation();
            setClosed(true);
            return true;
          }
        }
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          if (s.canSend) s.onSubmit();
          return true;
        }
        if (e.key === "Enter" && !e.shiftKey && !e.altKey && s.submitOn === "enter") {
          // A list or a code block keeps its own Enter; ⌘↵ sends from there.
          const inBlock = editorRef.current?.isActive("bulletList") || editorRef.current?.isActive("orderedList") || editorRef.current?.isActive("codeBlock");
          if (!inBlock) {
            if (s.canSend) s.onSubmit();
            return true;
          }
        }
        if (e.key === "Escape" && !s.wide && s.onEscape) {
          s.onEscape();
          return true;
        }
        void view;
        return false;
      },
    },
    onFocus: () => live.current.onFocus?.(),
    onUpdate: ({ editor }) => {
      // Keep the serializer's escaping intact, especially literal backslashes in code.
      last.current = editor.getMarkdown();
      setClosed(false);
      setIndex(0);
      onChange(last.current);
    },
  });
  const editorRef = useRef(editor);
  editorRef.current = editor;

  // Which token before the caret the list is offered for.
  const sync = () => {
    const s = live.current.suggest;
    const ed = editorRef.current;
    if (!s || !ed) return setMatch(null);
    const { $from } = ed.state.selection;
    setMatch(s.match($from.parent.textBetween(0, $from.parentOffset, undefined, "￼")));
  };
  useEffect(() => {
    if (!editor) return;
    editor.on("selectionUpdate", sync);
    editor.on("update", sync);
    return () => {
      editor.off("selectionUpdate", sync);
      editor.off("update", sync);
    };
  }, [editor]);

  function pick(item: Suggestion | undefined) {
    const ed = editorRef.current;
    const m = live.current.match;
    if (!ed || !item || !m) return;
    const { $from } = ed.state.selection;
    const from = $from.start() + m.start;
    ed.chain().focus().deleteRange({ from, to: $from.pos }).insertContent(item.insert).run();
    setIndex(0);
  }

  // The caller clears the text after it is sent; anything else that changes it comes from the editor.
  useEffect(() => {
    if (editor && value !== last.current) {
      editor.commands.setContent(value, { contentType: "markdown", emitUpdate: false });
      last.current = value;
    }
  }, [editor, value]);

  // The field names the active option, as the textbox pattern has it.
  useEffect(() => {
    // The view exists once the editor is mounted; before that reading it throws.
    const dom = editor?.isInitialized ? editor.view.dom : undefined;
    if (!dom) return;
    const set = (name: string, v?: string) => (v === undefined ? dom.removeAttribute(name) : dom.setAttribute(name, v));
    set("aria-autocomplete", suggest ? "list" : undefined);
    set("aria-controls", open ? listId : undefined);
    set("aria-activedescendant", open ? optionId(active) : undefined);
  });

  useImperativeHandle(handle, () => ({ focus: () => editor?.commands.focus("end") }), [editor]);

  // A widened box covers the card or page and follows it while the window changes.
  useLayoutEffect(() => {
    if (!wide) return;
    const fit = () => setRect(coverRect(root.current));
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, [wide]);

  const sendTip = sendTitle ?? sendLabel;
  return (
    <div
      ref={root}
      className={cx("mbx", wide && "wide", urgent && "urgent", className)}
      style={{ "--mbx-min": `${minHeight}px`, ...(wide ? rect : undefined) } as CSSProperties}
      // The first Escape narrows the box (or closes the list); the dialog around it is told to leave it be.
      data-escape-own={wide || open ? "" : undefined}
      onKeyDown={(e) => {
        if (wide && e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          setWide(false);
        }
      }}
    >
      {open && suggest && (
        <div className="mbx-suggest" role="listbox" id={listId} aria-label={suggest.label}>
          {options.map((o, i) => (
            <div key={o.key} id={optionId(i)} role="option" aria-selected={i === active} className="mbx-option" onMouseDown={(e) => e.preventDefault()} onMouseEnter={() => setIndex(i)} onClick={() => pick(o)}>
              {o.label}
            </div>
          ))}
        </div>
      )}
      {top && <div className="mbx-top">{top}</div>}
      <div className="mbx-field">
        <EditorContent editor={editor} />
        {!value.trim() && (
          <span className="mbx-placeholder" aria-hidden="true">
            {placeholder}
          </span>
        )}
      </div>
      <button
        type="button"
        className="mbx-wide"
        aria-pressed={wide}
        aria-label="Развернуть на всю область"
        title={wide ? "Свернуть" : "Развернуть"}
        onClick={() => {
          setWide(!wide);
          requestAnimationFrame(() => editor?.commands.focus("end"));
        }}
      >
        {wide ? <Icon.collapse size={15} /> : <Icon.expand size={15} />}
      </button>
      <div className="mbx-bar">
        {footer}
        <span className="mbx-grow" />
        <button type="button" className="mbx-send" disabled={!canSend} aria-label={sendLabel} title={sendTip} onClick={onSubmit}>
          <Icon.send size={16} />
        </button>
      </div>
    </div>
  );
}
