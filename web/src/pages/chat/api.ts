import { useQuery } from "@tanstack/react-query";
import { request, useInvalidating, type KeyPrefix } from "@/shared/api";
import type { ChatMessage } from "@/shared/api/generated/ChatMessage.ts";
import type { ChatPage } from "@/shared/api/generated/ChatPage.ts";
import { CHAT_PAGE, managerAnswering } from "./model.ts";

/** Every chat query starts with `chat`: the journal's `chat.posted` refetches them (shared/api/invalidation.ts). */
const CHAT: KeyPrefix[] = [["chat"]];

/** While the manager is answering, the newest page is polled; otherwise the live stream is enough. */
const ANSWER_POLL_MS = 3000;

export const useChatLatest = () =>
  useQuery({
    queryKey: ["chat", "latest"],
    queryFn: () => request<ChatPage>("GET", `/api/chat?limit=${CHAT_PAGE}`),
    refetchInterval: (query) => (managerAnswering(query.state.data?.messages ?? [], query.state.data?.manager) ? ANSWER_POLL_MS : false),
  });

/** The page of messages older than `before` (the id of the oldest one shown). */
export const loadOlderChat = (before: number) => request<ChatPage>("GET", `/api/chat?before=${before}&limit=${CHAT_PAGE}`);

export const useSendChat = () => useInvalidating((body: { text: string; replyTo?: number }) => request<ChatMessage>("POST", "/api/chat", body), CHAT);

/**
 * Starts the project manager ahead of a message, so its first reply is not waiting on a cold start. The
 * server answers whether it started anything and limits the calls itself: a failure is not news to the page.
 */
export function warmManager(): void {
  request<{ started: boolean }>("POST", "/api/chat/warm").catch(() => undefined);
}
