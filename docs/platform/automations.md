---
title: Автоматизации genie — правила, запуски и агентные задания
summary: Модель автоматизаций genie server в том виде, как она реализована — JSON-правила с триггерами (событие, расписание, ручной запуск, webhook), шаги (задачи, уведомления, агентные задания, команды, вопросы людям, ожидание, чейнджлог, релиз, http), долговечные запуски, лимиты и защита от каскадов; плейбуки обоих сценариев владельца.
type: reference
status: current
tags: [платформа, автоматизации, триггеры, агенты, каналы]
aliases: [automations, triggers, rules, playbooks, workflows]
paths: [crates/genie-core/src/automation.rs, crates/genie/src/engine.rs, crates/genie/src/http/automations.rs]
verified: 2026-10-01
---

# Автоматизации genie

Контекст — [[platform/vision]]; запуск и настройка — [[platform/getting-started]]. Движок читает журнал событий каждого проекта ([[platform/backend]]), расходы на модели ограничивает корпоративный провайдер (PD7 в [[platform/decisions]]).

## Правило

Правило — JSON (в вебе: «Автоматизации» → «Новое правило» или плейбук; в командной строке — `genie automation create --file rule.json`, плейбук — `genie automation install <id>`):

```json
{
  "name": "Задача закрыта — знания и чейнджлог",
  "on": { "event": "task.status_changed", "where": { "to": "done", "task.type": ["task", "bug"] } },
  "limits": { "concurrency": 2, "maxRunsPerHour": 20 },
  "dryRun": false,
  "steps": [
    { "id": "docs", "timeout": "45m", "agent": { "role": "documenter", "goal": "…", "output": { "summary": "…" } } },
    { "id": "tell", "notify": { "to": ["task.author"], "title": "{{ event.task.id }} готова", "text": "{{ steps.docs.output.summary }}" } }
  ]
}
```

## Триггеры

| `on` | Когда | Контекст шаблонов |
|---|---|---|
| `{"event": "task.status_changed", "where": {…}}` | событие журнала проекта; `task.*` — префиксом | поля события на верхнем уровне (`to`, `from`, `note`…), `task` (задача целиком), `actor.name/role/project_role`, `event` |
| `{"schedule": "0 9 * * 1-5", "tz": "Europe/Moscow"}` | cron: 5 полей (или 6 с секундами) | `schedule.at` |
| `{"manual": true}` | кнопка «Запустить», `genie automation run <id> --task G-7 [--input ключ=значение]` или `POST /api/automations/<id>/run {"task": "G-7"}` | `inputs`, `task` |
| `{"webhook": {"secret": "не короче 16 символов"}, "where": {…}}` | `POST /api/hooks/<id>?token=<secret>` | `payload` |

События журнала: `task.created`, `task.updated`, `task.status_changed`, `task.commented`, `task.criterion_checked`, `task.artifact_added`, `task.blocked`, `task.unblocked`, `task.team_assigned`, `team.spawned`, `team.stopped`, `mail.sent`, `doc.changed`, `doc.proposal`, `doc.proposal_decided`, `release.published`.

Фильтр `where` — пути через точку и значения: равенство, список (любое из), объект операций `not`, `exists`, `contains`, `not_contains`, `gt`, `lt`, `prefix`.

## Шаги

Шаблоны `{{ путь | фильтр }}` подставляются в любые строки шага; строка, целиком состоящая из одной подстановки, сохраняет тип (массив остаётся массивом). Фильтры: `length`, `json`, `lines`, `upper`. Общие поля шага: `id`, `if` (шаблон: пустое, `0`, `false`, `[]` — ложь), `retry`, `timeout` (`30s`, `15m`, `24h`, `2d`), `onError` (`fail` | `continue`). Выход шага доступен как `steps.<id>.output`.

| Шаг | Что делает | Выход |
|---|---|---|
| `task.status` | `{to, note, force, task}` — от имени `automation:<правило>:<запуск>` по правилам оркестратора: DoR/DoD, в проекте `assisted` не закрывает, `review`/`done` — только при порядке в доставке (запрос открыт, проверки не упали) | `{task, status}` |
| `task.comment` | `{text, kind, task}`; `@login` уведомляет человека проекта | `{task}` |
| `task.create` | `{title, description, acceptance, type, parent, labels, priority, deps, plan, inbox}` | `{id}` |
| `task.update` | поля `PATCH /api/tasks/<id>` (`description`, `plan`, `appendNotes`, `addAcceptance`, `labels`, `priority`…) и `task`; неизвестное поле — ошибка шага | `{task}` |
| `task.get` | текущее состояние задачи | задача |
| `task.ready` | `{task}` — задачу и те, что от неё зависят, из `draft`/`refining` в `ready`, если их ничего не держит: нет блока, зависимости закрыты, нет `needs_owner`, открытых вопросов, идущих заданий и работающей команды, выполнен DoR | `{moved: [id], held: [{task, holds}]}` |
| `notify` | `{to, title, text, link, channels}` — веб-центр + Telegram или почта | `{recipients, queued}` |
| `agent` | разовое агентное задание `{role, goal, inputs, output, workspace, model}`; ждёт результата `genie job output` | результат агента |
| `team` | собрать команду `{template, note, members, waitFor: ["review", "done"]}` | `{team, status}` |
| `ask` | вопросы человеку `{to, from, questions, remindAfter, timeout, onTimeout: needs_owner / continue / fail}`; ждёт ответов | `{answers: [{n, question, answer}]}` |
| `wait` | `{for: "24h"}` | `{}` |
| `wake_orchestrator` | письмо оркестратору `{text, task}` | `{}` |
| `changelog.add` | `{group: added / changed / fixed…, text, task}` → `Unreleased` чейнджлога проекта | `{added, path}` |
| `release` | `{version}` → версия чейнджлога и событие `release.published` | `{version, notes}` |
| `http` | `{url, method, headers, body}` | `{status, body}` |

Получатели `to`: `event.actor`, `task.author`, `task.assignees`, `project.owners`, `project.admins`, `project.members`, `@login`.

Агентные задания запускаются тем же механизмом, что участники команд (ход харнесса с промптом, навыками, MCP и правилами роли и командной строкой `genie`). `workspace`: `read-only` — репозиторий проекта без инструментов записи; `worktree` — свой git-worktree `job-<id>` на ветке `genie/job-<id>` (повторные попытки работают в нём же, после задания он остаётся с веткой-результатом); `scratch` и `none` — пустой каталог задания. Задание без `genie job output` считается неуспешным и повторяется до `runtime.maxAttempts`; задание, которое не удалось запустить (роль удалена, worktree не создался), — тоже.

## Надёжность

- **Ровно один запуск на событие**: запуск уникален по `(правило, ключ триггера)`; курсор журнала сдвигается после создания запусков, так что повторное чтение не запускает правило дважды.
- **Долговечность**: состояние каждого шага хранится; ждущие шаги (задания, вопросы, команды, паузы) опрашиваются и переживают рестарт; спецификация фиксируется в запуске — правка правила не меняет уже идущие запуски.
- **Каскады**: действия правила не запускают его же; цепочка правил останавливается на глубине 3; события агентных заданий наследуют глубину своего запуска.
- **Лимиты**: `concurrency` (одновременных запусков правила), `maxRunsPerHour`, `dryRun` (запуск только фиксируется), отмена запуска в вебе или `genie automation cancel <запуск>` (отменяет и его задания).
- **Наблюдаемость**: «Автоматизации» → запуск → шаги с входом, выходом, ожиданием и ошибкой; в командной строке — `genie automation runs` и `genie automation run-show <запуск>`, задания — `genie job list` и `genie job show <id>`.

## Встроенные уведомления

Без правил: задача ждёт решения (`needs_owner` от агента) — автору и админам; задачу закрыл агент — автору; предложение в базу знаний — владельцам раздела (иначе админам проекта).

## Плейбуки

| Плейбук | Триггер | Шаги |
|---|---|---|
| Задача закрыта → знания, чейнджлог и уведомление | `task.status_changed` → `done` (без метки `no-docs`) | документатор (`read-only`) → `changelog.add` → `notify` автору и админам |
| Новая задача от человека → аналитик → вопросы автору | `task.created` во входящих от человека | `task.status` → refining, аналитик, черновик критериев комментарием, `ask` автору (напоминание 24 ч, срок 72 ч → needs_owner), `wake_orchestrator` |
| Решение не принято за сутки → напоминание | `task.status_changed` → `needs_owner` | `wait 24h`, `task.get`, `notify`, если решение всё ещё нужно |
| Задачу ничего не держит → «Готово к работе» (`auto-ready`) | `task.*`, задача в `draft`, `refining` или `done` | `task.ready`: сама задача или, когда закрылась зависимость, задачи, которые её ждали |
| «Готово к работе» → оркестратор запускает работу (`ready-start`) | `task.status_changed` → `ready`, кроме эпиков | `wake_orchestrator`: собрать команду и запустить работу; если работа уже идёт — ничего, если не хватает решения владельца (интеграция, репозитории) — спросить его |
| Команда остановилась → оркестратор берёт следующую готовую задачу (`ready-next`) | `team.stopped` | `wake_orchestrator`: взять из `ready` задачи без команды по приоритету и запустить, сколько позволяют лимиты; при отказе по лимиту остальные остаются в `ready` |

Лимиты работы (`limits` в `config.json`): `maxActiveTeams` — активных команд на проект (4 по умолчанию), `maxActiveTeamsPerEpic` — активных команд на задачи одного эпика (0 — без лимита). Сервер отказывает в команде сверх лимита, задача остаётся в `ready`. Плейбук `ready-next` будит оркестратора, когда команда остановилась: он берёт следующую задачу из очереди, поэтому поток задач идёт не быстрее лимитов, но и не застревает.

Оба сценария владельца проверяются сквозными тестами (`crates/genie/tests/scenarios.rs`): реальный HTTP, процессы агентов, движок и поддельный Telegram Bot API.
