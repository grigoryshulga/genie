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
