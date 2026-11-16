import { useEffect, useRef, useState } from "react";
import { Icon, IconButton, MessageBox, type MessageBoxHandle, type SuggestMatch } from "@/shared/ui";
import { excerpt, mentionAt, mentionOptions, type MentionOption } from "../model.ts";

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
 * The chat's composer: the shared message box, where Enter sends and Shift+Enter starts a line, and an `@` list of
 * the project manager and the members (the list follows the textbox ARIA pattern).
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
  const box = useRef<MessageBoxHandle>(null);

  // Choosing a message to reply to moves the cursor to the field, ready to type.
  const replyId = reply?.id;
  useEffect(() => {
    if (replyId !== undefined) box.current?.focus();
  }, [replyId]);

  const submit = async () => {
    const body = text.trim();
    if (!body || busy) return;
    if (await onSend(body)) setText((current) => (current.trim() === body ? "" : current));
  };

  const match = (before: string): SuggestMatch | null => {
    const at = mentionAt(before, before.length);
    const options: MentionOption[] = at ? mentionOptions(people, at.query) : [];
    return at && options.length ? { start: at.start, items: options.map((o) => ({ key: o.key, label: o.label, insert: `@${o.key} ` })) } : null;
  };

  return (
    <div className="chat-composer">
      <MessageBox
        handle={box}
        value={text}
        onChange={setText}
        onSubmit={() => void submit()}
        label="Текст сообщения"
        placeholder="Сообщение проекту. @pm — спросить менеджера проекта"
        canSend={!!text.trim() && !busy}
        submitOn="enter"
        suggest={{ label: "Кого упомянуть", match }}
        onEscape={reply ? onCancelReply : undefined}
        onFocus={onFocus}
        top={
          reply && (
            <div className="chat-reply-to">
              <span className="chat-reply-line">
                <span>В ответ</span> <b>{reply.author}</b> <span className="chat-reply-text">{excerpt(reply.text, 120)}</span>
              </span>
              <IconButton label="Отменить ответ" className="chat-reply-x" onClick={onCancelReply}>
                <Icon.close size={12} />
              </IconButton>
            </div>
          )
        }
      />
    </div>
  );
}
