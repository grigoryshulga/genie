import { useQuery } from "@tanstack/react-query";
import { request } from "@/shared/api";
import type { TelegramStatus } from "@/shared/api/generated/TelegramStatus.ts";
export type { TelegramLink } from "@/shared/api/generated/TelegramLink.ts";
export const telegramUrl = (project: string, path = "") => `/api/project/telegram${path}?project=${encodeURIComponent(project)}`;
export const useProjectTelegram = (project: string) =>
  useQuery({
    queryKey: ["project-telegram", project],
    queryFn: () => request<TelegramStatus>("GET", telegramUrl(project)),
    enabled: !!project,
    refetchInterval: 3000,
  });
