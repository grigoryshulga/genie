// The browser tests' data: one owner, one project, a handful of tasks across statuses.
// The browser tests read the same constants, so names and IDs stay in one place.

export const OWNER = { login: "owner", name: "Анна Смирнова", password: "ui-tests-password" };
export const PROJECT = { id: "demo", name: "Демо", prefix: "DM" };

export interface SeedTask {
  title: string;
  description?: string;
  acceptance?: string[];
  priority?: number;
  type?: "task" | "bug" | "spike" | "epic";
  labels?: string[];
  /** The epic, by its index in SEED_TASKS. */
  parent?: number;
  /** Statuses to walk through after creation, in order. */
  statuses?: string[];
  /** The note recorded with the last status; needs_owner requires the question for the owner. */
  note?: string;
}

// Created in this order on an empty project, so the first is DM-1.
export const SEED_TASKS: SeedTask[] = [
  { title: "Веб-интерфейс: проверка экранов", type: "epic", description: "Браузерные тесты ключевых экранов.", priority: 1 },
  {
    title: "Форма входа показывает ошибку при неверном пароле",
    description: "Пользователь видит понятное сообщение и может повторить попытку.",
    acceptance: ["Сообщение об ошибке видно под формой", "Поля не очищаются"],
    priority: 1,
    labels: ["auth"],
    parent: 0,
    statuses: ["refining", "ready", "in_progress"],
  },
  {
    title: "Список задач фильтруется по поиску",
    acceptance: ["Поиск сужает список", "Адрес хранит запрос"],
    priority: 2,
    labels: ["tasks"],
    parent: 0,
    statuses: ["refining", "ready"],
  },
  { title: "Кнопка закрытия диалога не реагирует на мобильном", type: "bug", priority: 0, labels: ["mobile"] },
  { title: "Изучить визуальную регрессию", type: "spike", priority: 3, statuses: ["draft"] },
  { title: "Обновить документацию по запуску", priority: 4, labels: ["docs"] },
];

// Extra tasks for the capture harness only (`npm run ui:capture`): together with SEED_TASKS they
// cover all eleven statuses, so a redesigned screen can be checked against every status icon.
// They are kept out of the baseline seed so the committed screenshots stay unchanged.
export const CAPTURE_TASKS: SeedTask[] = [
  { title: "Черновик: описать правила уведомлений", priority: 3, statuses: ["draft"] },
  { title: "Входящая идея: тёмная тема для документации", priority: 4 },
  { title: "Проверить сборку образа", priority: 2, parent: 0, statuses: ["refining", "ready", "in_progress", "review"] },
  { title: "Исправить порядок колонок на доске", type: "bug", priority: 1, parent: 0, statuses: ["refining", "ready", "in_progress", "review", "changes_requested"] },
  { title: "Добавить экспорт отчёта", priority: 2, parent: 0, statuses: ["refining", "ready", "in_progress", "review", "approved"] },
  { title: "Выбрать провайдера почты", priority: 1, statuses: ["refining", "ready", "in_progress", "needs_owner"], note: "Какого провайдера почты использовать для уведомлений?" },
  { title: "Обновить зависимости клиента", priority: 3, statuses: ["refining", "ready", "in_progress", "review", "approved", "done"] },
  { title: "Старая задача, отменена", priority: 4, statuses: ["cancelled"] },
];
