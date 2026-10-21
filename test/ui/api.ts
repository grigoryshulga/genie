// Reads what the server stored, to check that a click changed the data and not only the screen.

import { expect, type APIRequestContext } from "@playwright/test";
import { PROJECT } from "./seed.ts";

export interface StoredTask {
  id: string;
  title: string;
  status: string;
  description?: string;
  labels: string[];
  /** The epic the server saved; absent when the task stands on its own. */
  parent?: string;
  acceptance: { id: number; text: string; done: boolean }[];
}

export async function storedTask(request: APIRequestContext, id: string): Promise<StoredTask> {
  const res = await request.get(`/api/tasks/${encodeURIComponent(id)}?project=${PROJECT.id}`);
  expect(res.ok(), `GET /api/tasks/${id}: ${res.status()}`).toBe(true);
  return res.json();
}

/**
 * An extra epic, created through the API: a flow that needs a second target cannot take it from the
 * seed without moving the IDs the committed screenshots show.
 */
export async function createEpic(request: APIRequestContext, title: string): Promise<string> {
  // A write needs the app's own header: the server refuses cross-site form posts without it.
  const res = await request.post(`/api/tasks?project=${PROJECT.id}`, { data: { title, type: "epic" }, headers: { "x-genie": "1" } });
  expect(res.ok(), `POST /api/tasks: ${res.status()}`).toBe(true);
  const epic: { id: string } = await res.json();
  return epic.id;
}
