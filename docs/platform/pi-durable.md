---
title: Pi Durable (@earendil-works/pi-durable)
summary: Что такое pi-durable (пакет, не режим pi), чем его рантайм отличается от pi-coding-agent, совместимость API, требования, версии и что миграция значила бы для genie.
type: note
status: draft
tags: [платформа, pi, рантайм, миграция]
aliases: [pi-durable, durable runtime, Pi Durable]
paths: []
verified: 2026-10-14
---

# Pi Durable (`@earendil-works/pi-durable`)

Исследование по первоисточникам (pi.dev, earendil.com, GitHub `earendil-works/pi`, npm). Коротко: **pi-durable — это отдельная экспериментальная npm-библиотека, а не режим и не флаг `pi`**. Рантайм агентов genie (`@earendil-works/pi-coding-agent`) она не заменяет и в его зависимости не входит; миграция не требуется и пока не возможна «флагом» — это другой способ встраивания агента в приложение.

## 1. Что такое pi-durable

- Отдельный пакет [`@earendil-works/pi-durable`](https://www.npmjs.com/package/@earendil-works/pi-durable) из монорепозитория [`earendil-works/pi`](https://github.com/earendil-works/pi) (`packages/durable`), «Durable conversation, task, and document runtime for Pi». Актуальная версия **1.2.0** (2026-10-12), MIT, ~338 тыс. загрузок в неделю ([npm](https://www.npmjs.com/package/@earendil-works/pi-durable)).
- Это **библиотека для встраивания** («durable harness»): `Harness.open(storage, …)` из своего процесса. У пакета нет своего CLI-бинарника, а `pi` его не запускает: в зависимостях [`@earendil-works/pi-coding-agent@1.2.0`](https://www.npmjs.com/package/@earendil-works/pi-coding-agent) — `pi-ai`, `pi-mcp`, `pi-tui`, `pi-codemode`, `pi-agent-core` — **pi-durable отсутствует**.
- Выпущен 2026-10-01 одновременно с Pi 1.0 как «экспериментальный субстрат для долгоиграющих agentic-приложений» ([анонс Pi 1.0](https://earendil.com/posts/pi-1-0/)). Явно позиционируется как *не замена* coding agent: «It does not replace the Pi coding agent. It is a framework for building any agentic application» ([анонс Pi Durable](https://earendil.com/posts/pi-durable/)).
- Статус: **experimental**, «The API changes without notice between releases» ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md)). Минорные версии уже ломают API (breaking changes в 1.1.0 и 1.2.0 — [CHANGELOG](https://github.com/earendil-works/pi/blob/main/packages/durable/CHANGELOG.md)).
- Технический базис: [`@earendil-works/pi-ai`](https://github.com/earendil-works/pi/tree/main/packages/ai) (модели — общий с pi), [`@earendil-works/chord`](https://github.com/earendil-works/pi) (документы/состояние/коммиты) и `@earendil-works/pi-codemode` (QuickJS-песочница). Весь исходник ~15 тыс. строк TypeScript ([анонс](https://earendil.com/posts/pi-durable/)).

## 2. Чем отличается рантайм

| Ось | `pi` (coding agent, 1.x) | pi-durable (1.2.0) |
|---|---|---|
| Сессия | файл сессии JSONL в `--session-dir`, resume/fork по файлу ([sessions](https://pi.dev/docs/latest/sessions.md)) | «беседа» = транскрипт неизменяемых записей (`pi.user`, `pi.assistant`, `pi.tool-result`, …) в storage; форк без копирования истории |
| Хранилище | файловая система | подключаемые бэкенды: `MemoryStorage`, SQLite (WAL, `synchronous=NORMAL`), JSONL, SQLite внутри Cloudflare Durable Object ([README, Storage](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#storage)) |
| Падение процесса | сессия сохранена на границах записей; человек смотрит и говорит «continue» ([анонс](https://earendil.com/posts/pi-durable/)) | каждый шаг — **task с чекпойнтом до его выполнения**; новый процесс открывает storage и продолжает с последнего чекпойнта (`harness.resume()`); оборванный запрос модели отправляется снова; tool call повторяется только при `replay: "safe"`, иначе модель получает `interrupted` с накопленным выводом |
| Идемпотентность | — | `requestId` у submission даёт exactly-once: повтор после краша возвращает тот же submission ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#persist-and-resume)) |
| Гарантии storage | — | коммиты атомарны; SQLite переживает падение процесса, но новейшие коммиты могут теряться при потере питания/хоста; **один процесс владеет storage, межпроцессной блокировки нет** ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#storage)) |
| Клиенты | один человек в TUI; RPC/JSON — по одному клиенту-подпроцессу | «multiplayer»: любое число клиентов смотрят и рулят одну беседу (`viewState()`/`watch()` отдают изменения коммитов; опоздавший/переподключившийся стартует с текущего состояния) |
| Очередь входов | inbox сессии | `pi.inbox`: follow-up / steer (вставляется после текущего раунда инструментов) / write / reset; сбой прогона (run) не теряет очередь |
| Prompt cache | — | у каждой беседы свой постоянный UUIDv7 (`pi.provider`), передаётся в pi-ai как `sessionId`; переживает reopen/reset/compaction |
| Компакция | авто-компакция сессии | compaction — такой же durable task, идёт в фоне; сериализация не включает thinking |
| Своё состояние | — | «documents»: типизированный JSON рядом с транскриптом, меняется в тех же атомарных коммитах |

## 3. Как запускается

- **Своего CLI нет.** Встраивание в процесс:

```typescript
const harness = await Harness.open(await openNodeSqliteStorage("./agent.sqlite"),
  { models, registry, env }, context);
const root = await harness.root(context, { agent: { model: { provider: "openai", modelId: "…" } } });
const settled = await (await root.submit({ type: "input", content: "…", requestId: "job-42" }, context)).wait(context);
```

  ([README, Quick Start](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#quick-start))

- Аналоги привычных режимов — библиотечные, из примеров пакета: `18-print` (print mode: submit → print answer) и `19-json` (JSON mode: `watchEvents()` отдаёт события в стиле coding agent — `message_start`, `tool_execution_start`, …); для своих UI — `viewState()`/`watch()` с точными операциями коммитов ([README, Examples](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#examples)). RPC-протокола как в pi нет — вместо подпроцесса с JSONL вы встраиваете harness и раздаёте событийные потоки клиентам сами.
- **У `pi` ничего не отобрали**: `-p/--print`, `--mode text|json|rpc` никуда не делись и документированы в актуальной документации ([CLI](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/cli.md), [RPC mode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md)). Genie-способ запуска (`pi --mode rpc` + `pi --print`, [[platform/agent-bus]]) остаётся валидным.
- Эталон «coding agent поверх pi-durable» существовал только как **демо из исходников**: `packages/coding-agent/src/experimental/durable` и `…/vacation` в теге `v1.0.0` ([анонс, «Try it»](https://earendil.com/posts/pi-durable/)); в 1.2.0 экспериментальный стек из `src/experimental` удалён ([CHANGELOG coding-agent 1.2.0](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md)), в дереве тега v1.2.0 остался один `source-resolver.ts`.

## 4. API расширений и провайдеров (совместимость с pi 1.0.x)

- **Провайдеры — общие.** Тот же `@earendil-works/pi-ai`: `createModels()`, `models.setProvider(openaiProvider())`, схемы TypeBox (`Type` из `pi-ai`). Но всё, что живёт в `~/.pi` у CLI (`settings.json`, `models.json`, `auth.json`, `/login`), к pi-durable не относится — учётные данные подаёт ваше приложение ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#quick-start)).
- **Расширения — другой API.** `defineExtension({ name, tools, sections, hooks, wraps, tasks })` + `createRegistry()`; беседы хранят *имена* расширений, реестр можно менять на лету (hot reload). Система промпта собирается из секций перед каждым запросом, изменения фиксируются в транскрипте позиционно. Экосистемы coding agent — **skills, prompt templates, themes, pi-пакеты, `pi mcp` — в pi-durable отсутствуют**.
- **Инструменты**: `@earendil-works/pi-durable/tools` даёт `read`/`write`/`edit`/`bash` (+ `powershell` с 1.0.4) как расширение `CodingTools`; `defineTool` с TypeBox-схемой; `replay: "safe"` для повторяемых после краша вызовов; `callers` (кто зовёт: модель/инструменты) и `modelTools`; deferred-инструменты + расширение `tool_search` (BM25); вложенные вызовы `api.executeTool()`; `structuredOutput` со схемой ([README, Tools](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#tools)).
- **Code mode**: расширение `codemode` (`createCodemodeExtension`), QuickJS-песочница; скрипт детерминированно воспроизводится после рестарта по журналу результатов ([README, Code Mode](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#code-mode)).
- **MCP**: встроенного MCP-клиента/менеджера нет (нет `mcp.json`, `/mcp`, `--no-mcp`); инструменты MCP-сервера — обычные tools в реестре, для больших наборов README прямо советует namespaces вида `mcp__github__` + deferred + `tool_search` ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#tools)). Поднимать MCP-мост придётся самому (или через codemode/вложенные вызовы).
- **Хуки**: `beforeRequest`/`afterResponse`/`onYield`/`afterTools` на генерацию, `beforeTool`/`afterTool` на инструменты, `beforeCompact` на компакцию; решения, которые должны пережить рестарт, — через `api.memo()` (первая запись выигрывает) ([анонс](https://earendil.com/posts/pi-durable/)). Это не хуки расширений coding agent (`before_agent_start` и т.п.).
- **Субагенты** встроенных нет — это паттерн: беседа, которой владеет tool call (`ownership: { kind: "task", taskId }`), примеры 22–23 ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#abort-and-subagents)).

## 5. Требования

- **Node ≥ 22.19.0** (`engines` в [package.json](https://github.com/earendil-works/pi/blob/main/packages/durable/package.json)) — как у pi CLI. Ядро переносимо: SQLite/JSONL-бэкенды не используют Node API и работают на Bun и в Cloudflare Durable Objects ([анонс](https://earendil.com/posts/pi-durable/)).
- Зависимости (все запинены точно): `@earendil-works/chord` 1.2.0, `@earendil-works/pi-ai` 1.2.0, `@earendil-works/pi-codemode` 1.2.0, `@silvia-odwyer/photon` 0.3.3 (WASM-обработка изображений, грузится только при импорте), `typebox`, `diff` ([package.json](https://github.com/earendil-works/pi/blob/main/packages/durable/package.json)). TypeBox в руте стоит ~23 МБ RSS (≈4 МБ в бандле) ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#storage)).
- Внешние сервисы не нужны: storage — файл SQLite / каталог JSONL / память / DO SQLite; «один процесс — один storage». Для своих бэкендов — conformance-сьюты `registerStorageConformance` / `registerEnvConformance` ([README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md#storage)).
- Готовые обвязки со стороны платформ: Cloudflare Agents SDK — `PiHarness` (beta) поверх Durable Objects, `pi-durable`/`pi-ai` как optional peer deps ≥1.0 ([Cloudflare docs](https://developers.cloudflare.com/agents/harnesses/pi/)); Temporal — сторонний `pi-temporal`, но он оборачивает **coding agent**, а не pi-durable ([Temporal blog](https://temporal.io/blog/the-immortal-life-of-pi-running-the-pi-coding-agent-on-temporal)).

## 6. Версии и «миграция»

Официального migration guide нет — его и не может быть между CLI и библиотекой. Точки входа: [анонс, раздел «Try it»](https://earendil.com/posts/pi-durable/) (посоветован путь «показать агенту README + 30 примеров»), [README](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md), нормативный [spec.md](https://github.com/earendil-works/pi/blob/main/packages/durable/docs/spec.md), примеры `00`–`31` в `packages/durable/test/examples`.

Оба пакета выпускаются из одного монорепозитория **синхронно**, но независимо (coding agent от pi-durable не зависит ни в одной версии):

| Дата | `pi-coding-agent` | `pi-durable` | Заметно |
|---|---|---|---|
| 2026-10-01 | 1.0.0 | 1.0.0 | Pi 1.0 + Pi Durable; демо durable-coding-agent в `src/experimental` (только в исходниках, не в npm-пакете) |
| 2026-10-03 | 1.0.1 | 1.0.1 | |
| 2026-10-04 | 1.0.2 | 1.0.2 | версия, зафиксированная в genie (`package-lock.json`) |
| 2026-10-05 | 1.0.3 / 1.0.4 | 1.0.3 / 1.0.4 | durable: `powershell`, watch/среды |
| 2026-10-07 | 1.1.0 | 1.1.0 | durable: breaking по Storage-интерфейсу |
| 2026-10-12 | 1.2.0 | 1.2.0 | coding-agent: экспериментальный стек и зависимость от chord удалены; durable: breaking (`ToolTaskInput`-юнионы, `output`/`structuredOutput`, `SessionFailed`) |

([CHANGELOG durable](https://github.com/earendil-works/pi/blob/main/packages/durable/CHANGELOG.md), [CHANGELOG coding-agent](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md))

## 7. Что это значит для genie

Факты о текущей интеграции: агенты genie — долгоживущие подпроцессы `pi --mode rpc` с расширением genie-bus (`crates/genie/src/sessions.rs`, [[platform/agent-bus]]), разовые задания — `pi --print` на ход (`crates/genie/src/config.rs`), версия зафиксирована lock-файлом на 1.2.0 (обновлена с 1.0.2 2026-10-14, полный прогон живых тестов зелёный), в docker — Node 24 ([[platform/docker]]).

Дальше — **предположения**, не план:

- Сбой хода агента сегодня — повтор целого хода с backoff; чекпойнты pi-durable дали бы продолжение с места обрыва и «interrupted»-результаты вместо повторных внешних эффектов. Но genie уже решает это на уровне шины (письма, лизы, статусы), т.е. выгода — в основном на длинных ходах.
- «Multiplayer»-модель pi-durable (любое число клиентов смотрит и рулит беседу через коммит-поток) — это в точности сценарий веб-чата команд genie; теоретически она заменила бы RPC-супервизор и трансляцию событий. Цена: веб-бэкенд должен встраивать harness в Node-процессе (сегодня ядро сервера — Rust, pi — подпроцессы).
- Потеря экосистемы CLI: skills, темы, `pi-пакеты`, встроенный MCP (`createMcpExtension` из pi-coding-agent, которым пользуется genie-guard) и `pi mcp login` при переходе на pi-durable пришлось бы заменять своим кодом.
- Экспериментальный статус и темп breaking changes (два ломающих минора за 11 дней) делают встраивание сегодня дорогим в сопровождении. При этом Cloudflare PiHarness (beta) показывает, что экосистема вокруг pi-durable строится, хотя сама Earendil называет пакет экспериментальным.
- Вывод: **мигрировать некуда и незачем** — pi-durable не является следующей версией pi-coding-agent. Стоит следить за ним как за кандидатом на рантайм «агентов вне терминала» ([[platform/vision]], адаптер AgentRuntime) и за обратным потоком фич (уроки durable явно обещали переносить в coding agent).

## 8. Источники

- Анонсы: [Pi 1.0](https://earendil.com/posts/pi-1-0/), [Pi Durable](https://earendil.com/posts/pi-durable/) (earendil.com, 2026-10-01)
- Репозиторий: [README pi-durable](https://github.com/earendil-works/pi/blob/main/packages/durable/README.md), [CHANGELOG pi-durable](https://github.com/earendil-works/pi/blob/main/packages/durable/CHANGELOG.md), [spec.md](https://github.com/earendil-works/pi/blob/main/packages/durable/docs/spec.md), [package.json](https://github.com/earendil-works/pi/blob/main/packages/durable/package.json), [CHANGELOG coding-agent](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md)
- npm: [@earendil-works/pi-durable](https://www.npmjs.com/package/@earendil-works/pi-durable), [@earendil-works/pi-coding-agent](https://www.npmjs.com/package/@earendil-works/pi-coding-agent)
- Документация pi CLI (актуальна для 1.2.0): [Command Line](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/cli.md), [RPC Mode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md), [docs index](https://pi.dev/docs/latest)
- Платформенные обвязки: [Cloudflare Agents: Pi harness (beta)](https://developers.cloudflare.com/agents/harnesses/pi/), [Temporal: pi-temporal](https://temporal.io/blog/the-immortal-life-of-pi-running-the-pi-coding-agent-on-temporal) (сторонние)
