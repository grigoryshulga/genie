// The development stand's data (test/ui/stand.ts): three projects, four people with different roles, and tasks
// chosen to put every screen in an awkward state: all eleven statuses, long titles, long lists, markdown,
// blocked and waiting tasks, a finished epic, an English project, an empty project, text that must be escaped.
// The browser tests do not use it, so growing it never moves their committed screenshots.

import { CAPTURE_DOCS, CAPTURE_TASKS, FLOW_DOC, OWNER, PROJECT, SEED_TASKS, type SeedTask } from "./seed.ts";
import type { Doc } from "./seeding.ts";

export const PEOPLE = [
  { login: OWNER.login, name: OWNER.name, password: OWNER.password, admin: true },
  { login: "ivan", name: "Иван Петров", password: "stand-password", admin: false },
  { login: "chen", name: "Mei Chen", password: "stand-password", admin: false },
  { login: "guest", name: "Гость", password: "stand-password", admin: false },
];

export interface StandProject {
  id: string;
  name: string;
  prefix: string;
  members: [login: string, role: string][];
  tasks: SeedTask[];
  docs: Doc[];
}

// Indexes below are positions in the project's task list, so the first stand epic follows the shared seed.
const FIRST = SEED_TASKS.length + CAPTURE_TASKS.length;

const LONG_TITLE =
  "Очень длинное название задачи, которое проверяет перенос строк, обрезку на карточках доски и поведение заголовка в модальном окне на узком экране";

const demoExtra: SeedTask[] = [
  {
    title: "Релиз 2.0: подготовка и выкатка",
    type: "epic",
    priority: 1,
    labels: ["release", "planning"],
    description: [
      "Цель: выпустить версию 2.0 без простоя и без потери данных.",
      "",
      "## Что входит",
      "",
      "- миграция базы данных с откатом;",
      "- обновление документации и changelog;",
      "- прогон нагрузочного теста на staging.",
      "",
      "| Этап | Срок | Ответственный |",
      "| --- | --- | --- |",
      "| Заморозка | пятница | Иван |",
      "| Выкатка | понедельник | Анна |",
    ].join("\n"),
    acceptance: ["Все задачи эпика закрыты", "Откат проверен на копии данных", "Changelog опубликован", "Владелец принял релиз"],
    checked: [0],
    comments: [{ kind: "decision", text: "Выкатываем в понедельник утром, пятница — только заморозка." }],
  },
  {
    title: LONG_TITLE,
    parent: FIRST,
    priority: 1,
    labels: ["ui", "mobile"],
    statuses: ["refining", "ready", "in_progress"],
    description: "Проверяет, как длинный заголовок ведёт себя на доске, в списке, в карточке и в уведомлениях.",
    acceptance: ["Заголовок переносится, а не обрезается молча", "Подсказка показывает полный текст", "Вёрстка не ломается на 320 px"],
    checked: [0],
    comments: [
      { kind: "note", text: "Начал с доски: карточка растёт по высоте, а колонка — нет." },
      { kind: "progress", text: "Исправил перенос в списке. Осталась модалка." },
      { kind: "question", text: "@ivan, нужен ли тултип на мобильных, где нет hover?" },
      { kind: "decision", text: "Тултип только на устройствах с указателем." },
      { kind: "review", text: "Проверил на 320 px: заголовок помещается в три строки." },
      { kind: "handoff", text: "Передаю в ревью, скриншоты в артефактах." },
    ],
    artifacts: [
      { name: "screens.md", kind: "test-report", text: "# Скриншоты\n\n- 320 px — ок\n- 768 px — ок\n- 1440 px — ок\n" },
      { name: "wrap.diff", kind: "diff", text: "--- a/card.css\n+++ b/card.css\n@@ -1,3 +1,4 @@\n .title {\n-  white-space: nowrap;\n+  overflow-wrap: anywhere;\n }\n" },
    ],
  },
  {
    title: "Исправить гонку при одновременном переносе карточек",
    type: "bug",
    parent: FIRST,
    priority: 0,
    labels: ["регресс", "критично", "backend", "api", "sql", "безопасность", "hotfix", "клиент"],
    statuses: ["refining", "ready", "in_progress", "review"],
    description: "При перетаскивании двух карточек одновременно вторая теряется.\n\n```sql\nUPDATE tasks SET position = position + 1 WHERE lane = ?;\n```\n\nВоспроизводится в 1 из 5 попыток.",
    comments: [{ kind: "review", text: "Нужен тест на параллельный перенос, без него не принимаю." }],
  },
  {
    title: "Подготовить сценарий отката миграции",
    parent: FIRST,
    priority: 2,
    labels: ["db"],
    statuses: ["refining", "ready", "in_progress"],
    blocked: "Ждём копию продовых данных от команды эксплуатации",
    comments: [{ kind: "question", text: "@guest, когда будет доступна копия данных?" }],
  },
  {
    title: "Написать changelog",
    parent: FIRST,
    priority: 3,
    labels: ["docs"],
    statuses: ["refining", "ready", "in_progress", "review", "approved", "done"],
  },
  {
    title: "Выбрать способ уведомлять о релизе",
    parent: FIRST,
    priority: 1,
    statuses: ["refining", "ready", "in_progress", "needs_owner"],
    note: "Как сообщать пользователям о новой версии?",
    action: "ask-owner-question",
    options: ["Баннер в приложении", "Письмо", "Только changelog"],
    description: "Решение влияет на текст релиза и на сроки.",
  },
  {
    title: "Add a dark-mode toggle ⚙️ to the settings page",
    priority: 2,
    labels: ["ui", "settings"],
    statuses: ["refining", "ready"],
    description: "Users want to switch themes without changing the OS setting.\n\n```ts\nconst theme = localStorage.getItem('theme') ?? 'system';\n```\n\n> Respect `prefers-color-scheme` when the choice is `system`.",
    acceptance: ["Toggle persists across reloads", "Follows the system theme by default"],
  },
  {
    title: "Сводка по приёмке: десять критериев",
    priority: 2,
    statuses: ["refining", "ready", "in_progress"],
    acceptance: Array.from({ length: 10 }, (_, i) => `Критерий приёмки №${i + 1}: проверка поведения на отдельном сценарии`),
    checked: [0, 1, 2, 3],
  },
  {
    title: "Обсуждение: единый формат сообщений об ошибках",
    priority: 3,
    statuses: ["refining"],
    comments: Array.from({ length: 9 }, (_, i) => ({
      kind: (["note", "question", "decision", "progress", "note"] as const)[i % 5],
      text: `Реплика ${i + 1}: ${["давайте начнём с API", "@ivan, а что с кодами HTTP?", "берём RFC 9457", "черновик готов", "нужны примеры"][i % 5]}.`,
    })),
  },
  {
    title: "<script>alert('xss')</script> \"Кавычки\" & <b>теги</b> 🧪",
    priority: 4,
    labels: ["<i>escape</i>"],
    description: "<img src=x onerror=alert(1)> должно показываться как текст.\n\n[ссылка](javascript:alert(1))",
    comments: [{ kind: "note", text: "<script>document.title='pwned'</script>" }],
  },
  {
    title: "Отменённая идея: чат поддержки",
    priority: 4,
    statuses: ["refining", "cancelled"],
    comments: [{ kind: "decision", text: "Не делаем: поддержка идёт через почту." }],
  },
  {
    title: "Мобильное приложение: первая версия",
    type: "epic",
    priority: 2,
    description: "Приложение для просмотра доски и ответов на вопросы владельцу.",
    acceptance: ["Публикация в магазинах"],
  },
  ...["Каркас навигации", "Экран доски", "Ответ на вопрос владельца"].map(
    (title): SeedTask => ({
      title,
      parent: FIRST + 11,
      priority: 2,
      statuses: ["refining", "ready", "in_progress", "review", "approved", "done"],
    }),
  ),
  { title: "Push-уведомления", parent: FIRST + 11, priority: 2, statuses: ["refining", "ready", "in_progress"] },
];

const shopTasks: SeedTask[] = [
  { title: "Checkout redesign", type: "epic", priority: 1, labels: ["checkout"], description: "Cut checkout from five steps to two.", acceptance: ["Conversion up 5%", "No regression in payment errors"] },
  { title: "Add Apple Pay", parent: 0, priority: 1, labels: ["payments"], statuses: ["refining", "ready", "in_progress"], acceptance: ["Works on Safari and iOS", "Falls back to card form"] },
  { title: "Validate postal codes per country", parent: 0, priority: 2, labels: ["forms"], statuses: ["refining", "ready"] },
  { title: "Cart total flickers on slow connections", type: "bug", parent: 0, priority: 1, statuses: ["refining", "ready", "in_progress", "review"], comments: [{ kind: "review", text: "Debounce the total, don't hide it." }] },
  { title: "Save cart for signed-in users", parent: 0, priority: 3, statuses: ["refining", "ready", "in_progress", "review", "approved", "done"] },
  { title: "Spike: evaluate Stripe Link", type: "spike", priority: 3, statuses: ["draft"] },
  { title: "Choose a tax provider", priority: 1, statuses: ["refining", "ready", "in_progress", "needs_owner"], note: "Which tax provider should we integrate first?", action: "ask-owner-question", options: ["Avalara", "TaxJar", "Build in-house"] },
  { title: "Update the privacy policy text", priority: 4, labels: ["legal", "docs"] },
];

const shopDocs: Doc[] = [
  { path: "shop/guides/payments.md", text: "---\ntitle: Payments overview\ntype: guide\nstatus: current\nverified: 2026-09-01\ntags: [payments]\n---\n# Payments overview\n\nCards go through the primary gateway; wallets use tokenised redirects.\n\n## Providers\n\n| Provider | Cards | Wallets |\n| --- | --- | --- |\n| Stripe | yes | yes |\n| Adyen | yes | no |\n" },
  { path: "shop/decisions/search.md", text: "---\ntitle: Search engine\ntype: decision\nstatus: draft\n---\n# Search engine\n\nUndecided between Meilisearch and Postgres full-text.\n" },
];

export const STAND_PROJECTS: StandProject[] = [
  {
    ...PROJECT,
    members: [[OWNER.login, "owner"], ["ivan", "member"], ["chen", "member"], ["guest", "viewer"]],
    tasks: [...SEED_TASKS, ...CAPTURE_TASKS, ...demoExtra],
    docs: [FLOW_DOC, ...CAPTURE_DOCS],
  },
  { id: "shop", name: "Storefront", prefix: "SF", members: [[OWNER.login, "owner"], ["chen", "admin"]], tasks: shopTasks, docs: shopDocs },
  { id: "empty", name: "Пустой проект", prefix: "EM", members: [[OWNER.login, "owner"]], tasks: [], docs: [] },
];
