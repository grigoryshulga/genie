# Genie domain language

Management terms used by the [project and task manager lifecycle](docs/platform/project-and-task-managers.md).

| Term | Meaning |
| --- | --- |
| **Project manager** | Persistent project agent responsible for shared context, human planning, prepared tasks, and the project queue. Russian: менеджер проекта. |
| **Task manager** (orchestrator) | Agent responsible for one task from dispatch and team coordination through result acceptance and retrospective publication. Russian: менеджер задачи, оркестратор. |
| **Task brief** | Recorded goal, requirements, scope, acceptance criteria, constraints, and sources used to dispatch a task. Russian: постановка задачи. |
| **Clarification** | A task-linked question requiring a confirmed agreement or missing decision before some work can proceed. Russian: запрос на уточнение. |
| **Task retrospective** | Evidence-based review of task execution, with findings and proposed improvements to roles, prompts, and coordination. Russian: ретро задачи. |

Use “orchestrator” for the task manager in the new model. The runtime's project-level `Orchestrator` variant and mailbox retain their names for compatibility.

## Observation plane

Terms and Russian UI labels of the [observation plane](docs/platform/observation-plane.md). The web UI uses exactly these labels.

| Term | Meaning |
| --- | --- |
| **Outcome case** | Post-delivery observation of one or more delivered tasks, key `O-<n>`. Russian: наблюдение. |
| **Outcome contract** | Versioned expectations of a case: hypotheses, guardrails, windows, and sources. Russian: контракт наблюдения. |
| **Hypothesis** | An expected result that a `verify` case checks. Russian: гипотеза. |
| **Guardrail** | A limit that must not be crossed after delivery. Russian: ограничитель. |
| **Evidence record** | One structured measurement with provenance and trust level. Russian: измерение. |
| **Anchor** | The moment observation started; windows are measured from it. Russian: начало наблюдения. |
| **Trust level** | How an evidence record was produced: `measured` by a source, `reported` by a person, or `agent` by an agent. Russian: доверие. |
| **Incident** | An open guardrail violation of a case, key `I-<n>`. Russian: инцидент. |
| **Follow-up task** | A delivery task created from an incident or a conclusion. Russian: задача по итогам наблюдения. |
| **Observation source** | A project's configured Prometheus- or Sentry-compatible server that cases read measurements from; its secret stays on the server. Russian: источник наблюдения. |

| UI key | Value | Russian label |
| --- | --- | --- |
| `status` | `proposed` | Черновик |
| `status` | `waiting` | Ждёт поставки |
| `status` | `observing` | Наблюдается |
| `status` | `evaluating` | Оценивается |
| `status` | `concluded` | Завершено |
| `status` | `cancelled` | Отменено |
| `result` | `confirmed` | Подтверждено |
| `result` | `negative` | Не подтверждено |
| `result` | `inconclusive` | Нет вывода |
| `policy` | `guardrails` | Ограничители |
| `policy` | `verify` | Проверка результата |
| `incident` | `open` | Открыт |
| `incident` | `resolved` | Разрешён |
| `incident` | `dismissed` | Отклонён |
| `kind` | `prometheus` | Prometheus-совместимый |
| `kind` | `sentry` | Sentry-совместимый |
| `auth` | `none` | Без авторизации |
| `auth` | `bearer` | Токен (Bearer) |
| `auth` | `basic` | Логин и пароль (Basic) |
| `nav` | sidebar item and list page title | Наблюдения |
| `section` | task card section and settings section title | Наблюдение |
| `empty` | task card without cases | Наблюдение не настроено |
| `create` | task card button | Создать наблюдение |
| `newTitle` | default title of a case created from a task | `<task title>` — наблюдение |
| `filter` | `active` | Активные |
| `filter` | `concluded` | Завершённые |
| `filter` | `all` | Все |
| `verdict` | `confirmed` | Подтверждено |
| `verdict` | `negative` | Не подтверждено |
| `verdict` | `inconclusive` | Нет вывода |
| `verdict` | `insufficient` | Недостаточно данных |
| `trigger` | `window` | по расписанию |
| `trigger` | `manual` | обычная |
| `trigger` | `final` | итоговая |
| `confidence` | `low` | низкая |
| `confidence` | `medium` | средняя |
| `confidence` | `high` | высокая |
| `criterion` | `met` | достигнуто |
| `criterion` | `not_met` | не достигнуто |
| `criterion` | `ok` | в норме |
| `criterion` | `violated` | нарушено |
| `criterion` | `insufficient` | недостаточно данных |
| `phase` | `baseline` | до поставки |
| `phase` | `observed` | после поставки |
| `trust` | `measured` | измерено |
| `trust` | `reported` | сообщено |
| `trust` | `agent` | от агента |
| `start` | `tasks_done` | выполненные задачи |
| `start` | `deployment` | развёртывание |
| `start` | `manual` | вручную |
| `action` | `rename` | Переименовать |
| `action` | `edit` | Изменить |
| `action` | `contract` | Задать контракт |
| `action` | `reviseContract` | Изменить контракт |
| `action` | `arm` | Ждать поставки |
| `action` | `start` | Начать наблюдение |
| `action` | `deploy` | Записать развёртывание |
| `action` | `evaluate` | Запросить оценку |
| `action` | `evaluateFinal` | Итоговая оценка |
| `action` | `evidence` | Записать измерение |
| `action` | `followup` | Создать задачу |
| `action` | `cancel` | Отменить наблюдение |
| `incidentAction` | `resolve` | Разрешить |
| `incidentAction` | `dismiss` | Отклонить |
| `incidentAction` | `followup` | Создать задачу |
| `heading` | `contract` | Контракт |
| `heading` | `deployments` | Развёртывания |
| `heading` | `evaluations` | Оценки |
| `heading` | `evidence` | Измерения |
| `heading` | `incidents` | Инциденты |
| `heading` | `followups` | Задачи по итогам |
| `field` | `title` | Название |
| `field` | `policy` | Политика |
| `field` | `tasks` | Задачи |
| `field` | `status` | Статус |
| `field` | `observationStart` | Начало наблюдения |
| `field` | `nextEvaluation` | Следующая оценка |
| `field` | `nextGuardrail` | Следующая проверка ограничителей |
| `field` | `evaluationRequested` | Оценка запрошена |
| `field` | `confidence` | Уверенность |
| `field` | `conclusion` | Вывод |
| `field` | `cancelReason` | Причина отмены |
| `field` | `ref` | Версия |
| `field` | `environment` | Окружение |
| `field` | `at` | Время |
| `field` | `recordedBy` | Записал |
| `field` | `criterion` | Критерий |
| `field` | `phase` | Фаза |
| `field` | `value` | Значение |
| `field` | `baseline` | База |
| `field` | `change` | Изменение |
| `field` | `samples` | Выборка |
| `field` | `source` | Источник |
| `field` | `trust` | Доверие |
| `field` | `note` | Примечание |
| `field` | `reason` | Причина |
| `field` | `description` | Описание |
| `field` | `contractJson` | Контракт (JSON) |
| `field` | `startOn` | Старт |
