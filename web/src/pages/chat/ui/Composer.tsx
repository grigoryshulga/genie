import { type KeyboardEvent, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Button, Icon, IconButton, Textarea } from "@/shared/ui";
import { excerpt, insertMention, mentionAt, mentionOptions, type MentionAt, type MentionOption, nextOption } from "../model.ts";

export interface ChatPerson {
  login: string;
  name: string;
}

/** The message a reply answers: its author's label and its text, for the quote above the field. */
export interface ReplyTarget {
  id: number;
  author: string;
  text: string;
}

/**
 * The chat's composer: a multi-line field, Enter sends and Shift+Enter starts a line, and an `@` list of the
 * project manager and the members. The list follows the textbox ARIA pattern: the field names the active
 * option with `aria-activedescendant`, and arrows, Enter/Tab and Escape drive it.
 */
export function Composer({
  people,
  reply,
  onCancelReply,
  onSend,
  busy,
  onFocus,
}: {
  people: ChatPerson[];
  reply?: ReplyTarget;
  onCancelReply: () => void;
  /** Resolves true once the server has the message, which clears the draft; false keeps it for another try. */
  onSend: (text: string) => Promise<boolean>;
  busy: boolean;
  onFocus: () => void;
}) {
  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  const [index, setIndex] = useState(0);
  // Escape closes the list until the next keystroke; moving the caret does not reopen it.
  const [closed, setClosed] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const caretAfterPick = useRef<number | null>(null);
  const listId = useId();

  const at = mentionAt(text, caret);
  const options = at ? mentionOptions(people, at.query) : [];
  const open = !!at && options.length > 0 && !closed;
  const active = Math.min(index, Math.max(0, options.length - 1));
  const optionId = (i: number) => `${listId}-${i}`;

  // Choosing a message to reply to moves the cursor to the field, ready to type.
  const replyId = reply?.id;
  useEffect(() => {
    if (replyId !== undefined) field.current?.focus();
  }, [replyId]);

  // A picked mention moves the caret past its inserted text, once the new value is in the field.
  useLayoutEffect(() => {
    if (caretAfterPick.current === null) return;
    field.current?.setSelectionRange(caretAfterPick.current, caretAfterPick.current);
    caretAfterPick.current = null;
  }, [text]);

  const pick = (option: MentionOption, token: MentionAt) => {
    const next = insertMention(text, caret, token, option.key);
    caretAfterPick.current = next.caret;
    setText(next.text);
    setCaret(next.caret);
    setIndex(0);
  };

  const submit = async () => {
    const body = text.trim();
    if (!body || busy) return;
    if (await onSend(body)) {
      setText((current) => (current.trim() === body ? "" : current));
      setCaret(0);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (open && at) {
      const option = options[active];
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setIndex(nextOption(active, options.length, e.key === "ArrowDown" ? 1 : -1));
        return;
      }
      if (option && ((e.key === "Enter" && !e.shiftKey) || (e.key === "Tab" && !e.shiftKey))) {
        e.preventDefault();
        pick(option, at);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setClosed(true);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape" && reply) {
      onCancelReply();
    }
  };

  return (
    <form
      className="chat-composer"
      aria-label="Новое сообщение"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      {reply && (
        <div className="chat-reply-to">
          <span className="chat-reply-line">
            <span>В ответ</span> <b>{reply.author}</b> <span className="chat-reply-text">{excerpt(reply.text, 120)}</span>
          </span>
          <IconButton label="Отменить ответ" className="chat-reply-x" onClick={onCancelReply}>
            <Icon.close size={12} />
          </IconButton>
        </div>
      )}
      <div className="chat-box">
        {open && at && (
          <div className="chat-mentions" role="listbox" id={listId} aria-label="Кого упомянуть">
            {options.map((option, i) => (
              <div
                key={option.key}
                id={optionId(i)}
                role="option"
                aria-selected={i === active}
                className="chat-mention"
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => setIndex(i)}
                onClick={() => pick(option, at)}
              >
                {option.label}
              </div>
            ))}
          </div>
        )}
        <Textarea
          ref={field}
          className="chat-input"
          aria-label="Текст сообщения"
          aria-autocomplete="list"
          aria-controls={open ? listId : undefined}
          aria-activedescendant={open ? optionId(active) : undefined}
          rows={2}
          value={text}
          placeholder="Сообщение проекту. @pm — спросить менеджера проекта"
          onChange={(e) => {
            setText(e.target.value);
            setCaret(e.target.selectionStart);
            setIndex(0);
            setClosed(false);
          }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
          onKeyDown={onKeyDown}
          onFocus={onFocus}
        />
        <div className="chat-row">
          <span className="chat-hint">Enter — отправить, Shift+Enter — новая строка</span>
          <span className="grow" />
          <Button type="submit" variant="primary" busy={busy} disabled={!text.trim()}>
            Отправить
          </Button>
        </div>
      </div>
    </form>
  );
}
