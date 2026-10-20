// Reads what the server stored, to check that a click changed the data and not only the screen.

import { expect, type APIRequestContext } from "@playwright/test";
import { PROJECT } from "./seed.ts";

export interface StoredTask {
  id: string;
  title: string;
  status: string;
  description?: string;
  labels: string[];
  acceptance: { id: number; text: string; done: boolean }[];
}

export async function storedTask(request: APIRequestContext, id: string): Promise<StoredTask> {
  const res = await request.get(`/api/tasks/${encodeURIComponent(id)}?project=${PROJECT.id}`);
  expect(res.ok(), `GET /api/tasks/${id}: ${res.status()}`).toBe(true);
  return res.json();
}
