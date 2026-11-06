// Small helpers shared by the documentation change screens: the title a changeset
// shows and the Russian wording of a server failure.

import type { Changeset } from "@/entities/doc";
import { changesetNumber, docActionError } from "@/entities/doc";

export function changesetTitle(changeset: Pick<Changeset, "id" | "title">): string {
  return changeset.title || `Изменения ${changesetNumber(changeset.id)}`;
}

export const failure = (error: unknown) =>
  docActionError({ status: (error as { status?: number } | undefined)?.status, message: error instanceof Error ? error.message : String(error) });
