import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { personName, useMembers } from "@/entities/project";
import { useSession } from "@/entities/session";
import {
  isFiltered,
  matchesBesidesStatus,
  parseFilter,
  PRESETS,
  PRIORITY_NAME,
  READINESS_NAME,
  type Readiness,
  sameSet,
  SORT_NAME,
  sortTasks,
  type Status,
  STATUS_NAME,
  STATUS_ORDER,
  StatusIcon,
  type TaskFilter,
  type TaskSort,
  TIME_NAME,
  type TimeRange,
  useTasks,
  writeFilter,
} from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import { isTyping, plural } from "@/shared/lib";
import { Button, ErrorState, Icon, IconButton, Kbd, useModalFocus } from "@/shared/ui";
import { ContextSearch, TeamsSummary } from "@/widgets/topbar";
import { TaskList } from "@/widgets/task-list";
import { FilterMenu, FilterOption } from "./FilterMenu.tsx";
// The task screen renders the task list rows, the shared toolbar/chip chrome and its filter sheet.
import "./tasks-sheets.css";

/** One choice inside a filter: a checkbox for sets, a radio for single choices. */
type Choice = { id: string; kind: "checkbox" | "radio"; checked: boolean; onChange: () => void; label: ReactNode; hint?: ReactNode };
/** One filter: a chip over its menu on a wide screen, a section of the sheet on a phone. */
type FilterGroup = { key: string; label: string; value: string; active: boolean; head?: string; extra?: ReactNode; choices: Choice[] };

/** «Задачи»: the project's tasks as a list, with search and filters by status, readiness, time and more. */
export function TasksPage({ searchRef }: { searchRef: React.RefObject<HTMLInputElement | null> }) {
  const [sp, setSp] = useSearchParams();
  const [focused, setFocused] = useState<string | undefined>();
  const [sheet, setSheet] = useState(false);
  const tasksQ = useTasks();
  const teams = useTeamMap();
  const session = useSession().data;
  const login = session?.mode === "users" ? session.user.login : undefined;
  const members = useMembers(login ? session?.project : undefined).data;
  const people = useMemo(() => new Map((members ?? []).map((m) => [m.user.login, personName(m.user)])), [members]);
  const selected = sp.get("task") ?? undefined;
  const f = parseFilter(sp);
  const set = (patch: Partial<TaskFilter>) => setSp(writeFilter({ ...f, ...patch }, sp), { replace: true });

  const all = tasksQ.data ?? [];
  const epics = all.filter((t) => t.type === "epic" && t.status !== "done" && t.status !== "cancelled");
  const rest = all.filter((t) => matchesBesidesStatus(t, f, login));
  const shown = useMemo(() => sortTasks(rest.filter((t) => f.statuses.includes(t.status)), f.sort), [rest, f.statuses, f.sort]);
  // Keyboard order follows the screen: by status groups when grouped.
  const ordered = useMemo(
    () => (f.grouped ? STATUS_ORDER.flatMap((s) => shown.filter((t) => t.status === s)) : shown),
    [shown, f.grouped],
  );
  const countOf = (statuses: Status[]) => rest.filter((t) => statuses.includes(t.status)).length;

  const open = (id: string) => {
    const next = new URLSearchParams(sp);
    next.set("task", id);
    setSp(next);
  };

  // j / k / Enter over the visible list
  const listRef = useRef(ordered);
  listRef.current = ordered;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e)) return;
      const list = listRef.current;
      const idx = list.findIndex((t) => t.id === focused);
      if (e.key === "j" || e.key === "ArrowDown") {
        setFocused(list[Math.min(list.length - 1, idx + 1)]?.id);
        e.preventDefault();
      } else if (e.key === "k" || e.key === "ArrowUp") {
        setFocused(list[Math.max(0, idx - 1)]?.id);
        e.preventDefault();
      } else if (e.key === "Enter" && focused) {
        open(focused);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // The filter sheet is the phone's filter rail in a dialog: it takes focus, keeps Tab inside and
  // hands focus back to the chip that opened it.
  const sheetRef = useRef<HTMLDivElement>(null);
  useModalFocus(sheetRef, sheet, () => setSheet(false));

  const toggle = <T,>(list: T[], v: T) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  const preset = PRESETS.find((p) => sameSet(p.statuses, f.statuses));
  const statusValue = preset ? preset.name.toLowerCase() : f.statuses.length === 1 ? STATUS_NAME[f.statuses[0]].toLowerCase() : `${f.statuses.length} из ${STATUS_ORDER.length}`;
  const whoName = f.who === "me" ? "я" : f.who === "none" ? "никто" : f.who ? (people.get(f.who) ?? f.who) : "все";
  const epicName = f.epic === "none" ? "без эпика" : f.epic ? f.epic : "любой";

  const filters: FilterGroup[] = [
    {
      key: "status",
      label: "Статус",
      value: statusValue,
      active: preset?.id !== "open",
      choices: STATUS_ORDER.map((s) => ({
        id: s,
        kind: "checkbox",
        checked: f.statuses.includes(s),
        onChange: () => set({ statuses: toggle(f.statuses, s) }),
        hint: countOf([s]) || undefined,
        label: (
          <>
            <StatusIcon status={s} size={12} /> {STATUS_NAME[s]}
          </>
        ),
      })),
    },
    {
      key: "ready",
      label: "Готовность",
      value: f.ready ? READINESS_NAME[f.ready] : "любая",
      active: !!f.ready,
      head: "По критериям приёмки",
      choices: [
        { id: "any", kind: "radio", checked: !f.ready, onChange: () => set({ ready: undefined }), label: "Любая" },
        ...(Object.keys(READINESS_NAME) as Readiness[]).map((r) => ({
          id: r,
          kind: "radio" as const,
          checked: f.ready === r,
          onChange: () => set({ ready: r }),
          hint: READINESS_HINT[r],
          label: cap(READINESS_NAME[r]),
        })),
      ],
    },
    {
      key: "time",
      label: f.by === "created" ? "Созданы" : "Обновлены",
      value: f.time ? TIME_NAME[f.time] : "за всё время",
      active: !!f.time,
      extra: (
        <div className="g-seg full" role="group" aria-label="По какой дате">
          <button type="button" aria-pressed={f.by === "updated"} onClick={() => set({ by: "updated" })}>
            Обновлены
          </button>
          <button type="button" aria-pressed={f.by === "created"} onClick={() => set({ by: "created" })}>
            Созданы
          </button>
        </div>
      ),
      choices: [
        { id: "all", kind: "radio", checked: !f.time, onChange: () => set({ time: undefined }), label: "За всё время" },
        ...(Object.keys(TIME_NAME) as TimeRange[]).map((r) => ({
          id: r,
          kind: "radio" as const,
          checked: f.time === r,
          onChange: () => set({ time: r }),
          label: cap(TIME_NAME[r]),
        })),
      ],
    },
    {
      key: "priority",
      label: "Приоритет",
      value: f.priorities.length ? f.priorities.map((p) => PRIORITY_NAME[p].toLowerCase()).join(", ") : "любой",
      active: f.priorities.length > 0,
      choices: PRIORITY_NAME.map((name, p) => ({
        id: String(p),
        kind: "checkbox",
        checked: f.priorities.includes(p),
        onChange: () => set({ priorities: toggle(f.priorities, p) }),
        label: name,
      })),
    },
    {
      key: "epic",
      label: "Эпик",
      value: epicName,
      active: !!f.epic,
      choices: [
        { id: "any", kind: "radio", checked: !f.epic, onChange: () => set({ epic: undefined }), label: "Любой" },
        { id: "none", kind: "radio", checked: f.epic === "none", onChange: () => set({ epic: "none" }), label: "Без эпика" },
        ...epics.map((e) => ({
          id: e.id,
          kind: "radio" as const,
          checked: f.epic === e.id,
          onChange: () => set({ epic: e.id }),
          hint: e.id,
          label: e.title,
        })),
      ],
    },
  ];
  if (login)
    filters.push({
      key: "who",
      label: "Ответственный",
      value: whoName,
      active: !!f.who,
      choices: [
        { id: "all", kind: "radio", checked: !f.who, onChange: () => set({ who: undefined }), label: "Все" },
        { id: "me", kind: "radio", checked: f.who === "me", onChange: () => set({ who: "me" }), label: "Я" },
        { id: "none", kind: "radio", checked: f.who === "none", onChange: () => set({ who: "none" }), label: "Никто не назначен" },
        ...[...people]
          .filter(([l]) => l !== login)
          .map(([l, name]) => ({ id: l, kind: "radio" as const, checked: f.who === l, onChange: () => set({ who: l }), label: name })),
      ],
    });

  const choice = (c: Choice) => (
    <FilterOption key={c.id} kind={c.kind} checked={c.checked} onChange={c.onChange} hint={c.hint}>
      {c.label}
    </FilterOption>
  );

  // What the headline says the list is: the active filters in words, or the whole project.
  const active: string[] = [];
  if (!preset && f.statuses.length) active.push(`статус: ${f.statuses.map((s) => STATUS_NAME[s].toLowerCase()).join(", ")}`);
  if (f.ready) active.push(`готовность: ${READINESS_NAME[f.ready].toLowerCase()}`);
  if (f.time) active.push(`${f.by === "created" ? "созданы" : "обновлены"} ${TIME_NAME[f.time].toLowerCase()}`);
  if (f.priorities.length) active.push(`приоритет: ${f.priorities.map((p) => PRIORITY_NAME[p].toLowerCase()).join(", ")}`);
  if (f.epic) active.push(f.epic === "none" ? "без эпика" : `эпик ${f.epic}`);
  if (f.who) active.push(`ответственный: ${whoName}`);

  const reset = () =>
    setSp(writeFilter(parseFilter(new URLSearchParams()), new URLSearchParams(selected ? { task: selected } : {})), { replace: true });

  const viewChips = (
    <>
      <label className="g-chip g-chip-select">
        <span className="k">Порядок</span>
        <select value={f.sort} onChange={(e) => set({ sort: e.target.value as TaskSort })}>
          {(Object.keys(SORT_NAME) as TaskSort[]).map((s) => (
            <option key={s} value={s}>
              {SORT_NAME[s]}
            </option>
          ))}
        </select>
      </label>
      <label className="g-chip g-chip-select">
        <span className="k">Группы</span>
        <select value={f.grouped ? "status" : "none"} onChange={(e) => set({ grouped: e.target.value === "status" })}>
          <option value="status">по статусу</option>
          <option value="none">без групп</option>
        </select>
      </label>
    </>
  );

  return (
    <main className="main g-tasks-page" aria-label="Задачи">
      <div className="g-toolbar" role="toolbar" aria-label="Фильтры и вид">
        <ContextSearch inputRef={searchRef} className="mobile-context-search" />
        <div className="g-filters g-rail d-only">
          {filters.map((g) => (
            <FilterMenu key={g.key} label={g.label} value={g.value} active={g.active}>
              {g.head && <span className="fhead">{g.head}</span>}
              {g.extra}
              {g.choices.map(choice)}
            </FilterMenu>
          ))}
        </div>
        <div className="m-only">
          <button type="button" className="g-chip" aria-expanded={sheet} aria-haspopup="dialog" onClick={() => setSheet(true)}>
            <Icon.list size={12} />
            Фильтры
            {active.length > 0 && <span className="g-tag solid-signal g-num">{active.length}</span>}
          </button>
        </div>
        <span className="grow" />
        <div className="d-only g-filters">{viewChips}</div>
        {isFiltered(f) && (
          <Button variant="ghost" sm onClick={reset}>
            Сбросить
          </Button>
        )}
      </div>

      {sheet && (
        <div className="g-scrim g-fsheet-scrim" role="presentation" onClick={() => setSheet(false)}>
          <div ref={sheetRef} className="g-fsheet" role="dialog" aria-modal="true" aria-label="Фильтры" onClick={(e) => e.stopPropagation()}>
            <header className="g-fsheet-head">
              <span className="g-h3">Фильтры</span>
              <IconButton label="Закрыть" onClick={() => setSheet(false)}>
                <Icon.close size={16} />
              </IconButton>
            </header>
            <div className="g-fsheet-body">
              {filters.map((g) => (
                <section key={g.key} className="g-fsheet-group">
                  <h3 className="g-eyebrow">{g.label}</h3>
                  {g.head && <p className="g-fsheet-note">{g.head}</p>}
                  {g.extra}
                  {g.choices.map(choice)}
                </section>
              ))}
              <section className="g-fsheet-group">
                <h3 className="g-eyebrow">Вид</h3>
                <div className="g-fsheet-row">{viewChips}</div>
              </section>
              {isFiltered(f) && (
                <Button variant="secondary" onClick={reset}>
                  Сбросить фильтры
                </Button>
              )}
            </div>
          </div>
        </div>
      )}

      {tasksQ.isPending ? (
        <div className="g-tasks-scroll" aria-busy="true">
          {[0, 1, 2, 3, 4, 5, 6].map((i) => (
            <div key={i} className="g-skeleton tl-skeleton" />
          ))}
        </div>
      ) : tasksQ.isError ? (
        <div className="g-tasks-scroll">
          <ErrorState message={`Не удалось загрузить задачи: ${tasksQ.error.message}`} onRetry={() => void tasksQ.refetch()} />
        </div>
      ) : (
        <div className="g-tasks-scroll">
          <TaskList
            tasks={shown}
            statuses={f.statuses}
            grouped={f.grouped}
            when={(f.time && f.by === "created") || f.sort === "created" ? "created" : "updated"}
            teams={teams}
            people={people}
            focused={focused}
            selected={selected}
            onOpen={open}
            onFocus={setFocused}
            empty={isFiltered(f) ? "Ничего не нашлось" : undefined}
          />
        </div>
      )}

      <footer className="g-hints">
        <span>
          <Kbd>J</Kbd> <Kbd>K</Kbd> навигация
        </span>
        <span>
          <Kbd>↵</Kbd> открыть
        </span>
        <span>
          <Kbd>/</Kbd> поиск
        </span>
        <span>
          <Kbd>G</Kbd> <Kbd>B</Kbd> доска
        </span>
        <span>
          <Kbd>⌘K</Kbd> команды
        </span>
        <span className="grow" />
        <TeamsSummary teams={teams} />
      </footer>
    </main>
  );
}

const READINESS_HINT: Record<Readiness, string> = { none: "критерии не записаны", zero: "0 из N", partial: "часть выполнена", full: "N из N" };

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
