// A running team on the development stand (test/ui/stand.ts): the stand runs without agents, so the team's
// mail and one agent's conversation are written the way a live team would have left them. The team page
// gets a chat with Markdown (headings, lists, tables, code), long commands and unread mail; the executor's
// chat reads its conversation from the session file, as it does after an idle session has stopped.

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { genie } from "./seeding.ts";
import { PROJECT } from "./seed.ts";

const PLAN = [
  "## План экспорта",
  "",
  "1. Запрос по периоду с фильтром по складу.",
  "2. Потоковая запись CSV, без загрузки всего отчёта в память.",
  "3. Кнопка «Скачать» на странице отчёта.",
  "",
  "| Колонка | Источник | Формат |",
  "| --- | --- | --- |",
  "| Дата | `orders.created_at` | `YYYY-MM-DD` |",
  "| Склад | `warehouses.name` | текст |",
  "| Сумма | `orders.total` | `12345.67` |",
  "",
  "Вопросы — в почту, отвечу быстро.",
].join("\n");

const LONG_COMMAND =
  "cd /data/workspaces/demo/report && cargo test -p reports --features csv-export -- export::streams_rows_without_buffering export::writes_bom_for_excel export::empty_period_gives_header_only --nocapture --test-threads=1";

export function seedTeam(bin: string, dataDir: string) {
  const run = (args: string[]) => genie(bin, ["--project", PROJECT.id, ...args], undefined, dataDir);
  const task: string = JSON.parse(
    run(["task", "create", "Экспорт отчёта о продажах в CSV", "--json", "--description", "Скачивание отчёта о продажах за период в CSV.", "--ac", "Файл открывается в Excel без кракозябр", "--ac", "Пустой период даёт файл с заголовком"]),
  ).id;
  for (const status of ["refining", "ready", "in_progress"]) run(["task", "status", status, "--task", task]);
  run(["team", "spawn", task, "--template", "full"]);
  const team: { id: string; members: { name: string; role: string }[] } = JSON.parse(run(["team", "show", task, "--json"]));
  const name = (role: string) => team.members.find((m) => m.role === role)!.name;
  const [analyst, executor] = [name("analyst"), name("executor")];

  const db = new DatabaseSync(join(dataDir, "projects", PROJECT.id, "genie.db"));
  const now = Date.now();
  const at = (minutesAgo: number) => new Date(now - minutesAgo * 60_000).toISOString();
  db.prepare("UPDATE mail SET at = ?, delivered_at = ? WHERE team = ?").run(at(42), at(42), team.id);
  const insert = db.prepare(
    "INSERT INTO mail(team, at, sender, sender_role, recipient, text, level, intent, kind, task, delivered_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'message', ?, ?)",
  );
  const mail = (minutesAgo: number, from: string, fromRole: string, to: string, text: string, opts: { level?: string; intent?: string; unread?: boolean } = {}) =>
    Number(insert.run(team.id, at(minutesAgo), from, fromRole, to, text, opts.level ?? "normal", opts.intent ?? null, task, opts.unread ? null : at(minutesAgo - 1)).lastInsertRowid);

  const plan = mail(40, analyst, "analyst", executor, PLAN, { intent: "done" });
  const scope = mail(36, "owner", "human", executor, "Давайте сначала только CSV, XLSX — отдельной задачей. Колонки как в таблице, порядок важен.", { level: "high" });
  mail(30, executor, "executor", analyst, "Для Excel нужен BOM в начале файла? Без него кириллица ломается при двойном клике.", { intent: "question" });
  mail(28, analyst, "analyst", executor, "Да, пиши `\\uFEFF` первым символом. Разделитель — точка с запятой, это ожидание бухгалтерии.");
  mail(6, "owner", "human", executor, "Не забудь тест на пустой период: файл с одной строкой заголовка, без ошибки.", { unread: true });
  db.prepare("UPDATE members SET activity = 'working', activity_at = ?, status = ? WHERE team = ? AND name = ?").run(at(1), "Пишет тесты экспорта", team.id, executor);
  db.close();

  // The executor's conversation as pi keeps it: one JSON entry per line, each pointing at its parent.
  const entries: object[] = [];
  let parent: string | null = null;
  const add = (minutesAgo: number, entry: Record<string, unknown>) => {
    const id = `e${entries.length}`;
    const stamp = now - minutesAgo * 60_000;
    entries.push(entry.type === "custom_message" ? { ...entry, id, parentId: parent, timestamp: new Date(stamp).toISOString() } : { type: "message", id, parentId: parent, message: { ...entry, timestamp: stamp } });
    parent = id;
  };
  const delivered = (minutesAgo: number, ids: number[], text: string) => add(minutesAgo, { type: "custom_message", customType: "genie-mail", content: text, display: true, details: { mailIds: ids } });
  let call = 0;
  const tool = (minutesAgo: number, name: string, args: Record<string, unknown>, output: string, error = false) => {
    const id = `call_${++call}`;
    add(minutesAgo, { role: "assistant", content: [{ type: "toolCall", id, name, arguments: args }] });
    add(minutesAgo - 0.2, { role: "toolResult", toolCallId: id, toolName: name, content: [{ type: "text", text: output }], isError: error });
  };
  add(41, { role: "user", content: [{ type: "text", text: "Your genie session has started; your mail follows." }] });
  delivered(39, [plan], `[genie mail · 1 new]\n\n${PLAN}`);
  add(38, {
    role: "assistant",
    content: [
      { type: "thinking", thinking: "План понятен. Сначала посмотрю, как устроен текущий отчёт и где его запрос." },
      { type: "text", text: "Беру план. Сначала разберусь, откуда отчёт берёт данные." },
    ],
  });
  tool(37, "bash", { command: "rg -n 'fn sales_report' --type rust crates/ && git log --oneline -5 -- crates/reports" }, "crates/reports/src/sales.rs:41:pub fn sales_report(db: &Db, period: Period) -> Result<Vec<Row>> {\n9f2c1aa reports: warehouse filter\n71b0e3d reports: totals per day");
  delivered(35, [scope], "[genie mail · 1 new]\n\nДавайте сначала только CSV, XLSX — отдельной задачей.");
  tool(33, "read", { path: "crates/reports/src/sales.rs" }, Array.from({ length: 40 }, (_, i) => `${i + 30}\t${i % 7 ? "    let rows = query.fetch(db)?;" : "// sales report"}`).join("\n"));
  add(31, { role: "assistant", content: [{ type: "text", text: "Уточню у аналитика кодировку, пока пишу запрос." }] });
  tool(24, "bash", { command: LONG_COMMAND }, "running 3 tests\ntest export::streams_rows_without_buffering ... ok\ntest export::writes_bom_for_excel ... ok\ntest export::empty_period_gives_header_only ... FAILED\n\nfailures:\n    export::empty_period_gives_header_only: expected 1 line, got 0", true);
  add(20, {
    role: "assistant",
    content: [
      {
        type: "text",
        text: [
          "Экспорт пишет строки потоком. Что готово:",
          "",
          "| Проверка | Итог |",
          "| --- | --- |",
          "| Потоковая запись | ✅ |",
          "| BOM для Excel | ✅ |",
          "| Пустой период | ❌ нет заголовка |",
          "",
          "Чиню пустой период: заголовок пишется до первой строки.",
          "",
          "```rust",
          "writer.write_record(HEADER)?; // before the first row, even when there is none",
          "```",
        ].join("\n"),
      },
    ],
  });
  // The step running now: a call without a result yet.
  add(2, { role: "assistant", content: [{ type: "toolCall", id: "call_now", name: "bash", arguments: { command: "cargo test -p reports export:: -- --nocapture" } }] });
  const sessions = join(dataDir, "sessions", PROJECT.id);
  mkdirSync(sessions, { recursive: true });
  const header = { type: "session", version: 3, id: `${team.id}-${executor}`.toLowerCase(), timestamp: new Date(now - 41 * 60_000).toISOString() };
  writeFileSync(join(sessions, `${header.timestamp.replace(/[:.]/g, "-")}_${header.id}.jsonl`), [header, ...entries].map((e) => JSON.stringify(e)).join("\n") + "\n");
  return task;
}
