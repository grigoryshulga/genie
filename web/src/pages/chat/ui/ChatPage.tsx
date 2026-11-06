import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useSession } from "@/entities/session";
import { useMembers } from "@/entities/project";
import { MessageText } from "@/entities/team";
import { clock } from "@/shared/lib";
import { Avatar, Button, cx, EmptyState, ErrorState, LoadingState, PageHeader, Skeleton, useToast, WorkingArcs } from "@/shared/ui";
import type { ChatMessage } from "@/shared/api/generated/ChatMessage.ts";
import { loadOlderChat, useChatLatest, useSendChat, warmManager } from "../api.ts";
import { CHAT_PAGE, type ChatRow, authorLabel, chatRows, excerpt, isManager, managerAnswering, mayHaveOlder, mergeMessages, quotedMessage } from "../model.ts";
import { Composer } from "./Composer.tsx";
import "./chat.css";

const MANAGER_LABEL = "Менеджер проекта";
/** Within this many pixels of the bottom, a new message scrolls the feed; further up the reader keeps their place. */
const BOTTOM_SLACK = 60;
/** Within this many pixels of the top, the next older page loads. */
const TOP_SLACK = 80;

/** The project chat. The session's project keys the screen, so switching projects starts a clean list. */
export function ProjectChat() {
  const session = useSession().data;
  return <ChatScreen key={session?.project ?? ""} login={session?.user.login} project={session?.project} />;
}

function ChatScreen({ login, project }: { login?: string; project?: string }) {
  const toast = useToast();
  const members = useMembers(project).data ?? [];
  const latest = useChatLatest();
  const send = useSendChat();
  // Older pages the reader scrolled up to; the newest page comes from the live query.
  const [older, setOlder] = useState<ChatMessage[]>([]);
  const [olderFull, setOlderFull] = useState<boolean>();
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [reply, setReply] = useState<ChatMessage>();
  const feed = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  // The feed's height before an older page was prepended, to keep the reader's place.
  const heightBefore = useRef<number | null>(null);
  const loading = useRef(false);

  const newest = latest.data?.messages ?? [];
  const messages = mergeMessages(older, newest);
  const rows = chatRows(messages);
  const answering = managerAnswering(messages, latest.data?.manager);
  const names = new Map(members.map((m) => [m.user.login, m.user.name || m.user.login]));
  const labelOf = (m: ChatMessage) => authorLabel(m, names, MANAGER_LABEL);
  const choices = members.filter((m) => !m.user.disabled).map((m) => ({ login: m.user.login, name: m.user.name || m.user.login }));
  const oldestId = messages[0]?.id;
  const newestId = messages[messages.length - 1]?.id;
  const mayLoadOlder = mayHaveOlder(newest.length >= CHAT_PAGE, olderFull);

  // The project manager starts while the reader is here, so its first reply does not wait for a cold start.
  useEffect(() => {
    warmManager();
  }, []);

  // A new message follows the conversation only when the reader is already at the bottom.
  useLayoutEffect(() => {
    const el = feed.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [newestId, answering]);

  // An older page was prepended above: move the scroll by the height it added, so the same message stays in view.
  useLayoutEffect(() => {
    const el = feed.current;
    if (!el || heightBefore.current === null) return;
    el.scrollTop += el.scrollHeight - heightBefore.current;
    heightBefore.current = null;
  }, [older]);

  const onScroll = () => {
    const el = feed.current;
    if (!el) return;
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_SLACK;
    if (el.scrollTop < TOP_SLACK) void loadOlder();
  };

  async function loadOlder() {
    if (loading.current || !mayLoadOlder || oldestId === undefined) return;
    loading.current = true;
    // Measured before the caption appears, so the caption's own height does not count as added messages.
    heightBefore.current = feed.current?.scrollHeight ?? null;
    setLoadingOlder(true);
    try {
      const page = await loadOlderChat(oldestId);
      setOlder((prev) => mergeMessages(page.messages, prev));
      setOlderFull(page.messages.length >= CHAT_PAGE);
    } catch (error) {
      heightBefore.current = null;
      toast(error instanceof Error ? error.message : "Не удалось загрузить сообщения", "error");
    } finally {
      loading.current = false;
      setLoadingOlder(false);
    }
  }

  // The composer clears its draft only when this resolves true; a failure keeps the text for another try.
  const sendMessage = async (text: string): Promise<boolean> => {
    try {
      await send.mutateAsync({ text, replyTo: reply?.id });
      setReply(undefined);
      atBottom.current = true;
      return true;
    } catch (error) {
      toast(error instanceof Error ? error.message : "Не удалось отправить сообщение", "error");
      return false;
    }
  };

  return (
    <main className="main chat-page" aria-label="Чат проекта">
      <PageHeader title="Чат" />
      <div className="chat-feed" ref={feed} role="log" aria-label="Сообщения проекта" onScroll={onScroll}>
        <div className="chat-inner">
          {loadingOlder && (
            <p className="chat-caption" role="status">
              Загрузка сообщений…
            </p>
          )}
          {latest.isPending ? (
            <LoadingState label="Загрузка чата" width={420}>
              <Skeleton width={200} height={14} />
              <Skeleton height={14} />
              <Skeleton height={14} width="72%" />
            </LoadingState>
          ) : latest.isError ? (
            <ErrorState message="Не удалось загрузить чат" onRetry={() => void latest.refetch()} />
          ) : messages.length === 0 ? (
            <EmptyState title="Сообщений пока нет">Тэгните @pm, чтобы спросить менеджера проекта: статус задач, создать или запустить задачу.</EmptyState>
          ) : (
            rows.map((row) => (
              <Message
                key={row.message.id}
                row={row}
                mine={row.message.authorRole === "human" && row.message.author === login}
                quote={quotedMessage(messages, row.message.replyTo)}
                labelOf={labelOf}
                onReply={() => setReply(row.message)}
              />
            ))
          )}
        </div>
      </div>
      {answering && (
        <div className="chat-typing" role="status">
          <WorkingArcs size={12} label="Работает" />
          <span>Менеджер проекта отвечает…</span>
        </div>
      )}
      <Composer
        people={choices}
        reply={reply && { id: reply.id, author: labelOf(reply), text: reply.text }}
        onCancelReply={() => setReply(undefined)}
        onSend={sendMessage}
        busy={send.isPending}
        onFocus={warmManager}
      />
    </main>
  );
}

/** One message: others' on the left under an avatar and a name, the person's own on the right in the accent. */
function Message({
  row,
  mine,
  quote,
  labelOf,
  onReply,
}: {
  row: ChatRow;
  mine: boolean;
  quote?: ChatMessage;
  labelOf: (m: ChatMessage) => string;
  onReply: () => void;
}) {
  const { message: m, continued } = row;
  const manager = isManager(m);
  const label = labelOf(m);
  return (
    <article className={cx("chat-msg", mine && "mine", manager && "manager", continued && "cont")}>
      {!mine && <span className="chat-slot">{!continued && <Avatar kind={manager ? "agent" : "human"} tone={manager ? "project-manager" : undefined} name={label} sm />}</span>}
      <div className="chat-body">
        {!mine && !continued && (
          <div className="chat-from">
            <b>{label}</b>
          </div>
        )}
        <div className={cx("chat-bubble", mine && "mine", continued && "cont")}>
          {m.replyTo !== null && (
            <div className="chat-quote">
              {quote ? (
                <>
                  <b>{labelOf(quote)}</b> <span>{excerpt(quote.text, 100)}</span>
                </>
              ) : (
                <span>Ответ на сообщение #{m.replyTo}</span>
              )}
            </div>
          )}
          <MessageText text={m.text} />
          <span className="chat-bubble-foot">
            <time dateTime={m.at}>{clock(m.at)}</time>
          </span>
        </div>
        <div className="chat-actions">
          <Button variant="ghost" sm className="chat-reply" onClick={onReply} aria-label={`Ответить на сообщение: ${excerpt(m.text, 60)}`}>
            Ответить
          </Button>
        </div>
      </div>
    </article>
  );
}
