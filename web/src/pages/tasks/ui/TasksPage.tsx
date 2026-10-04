import { useEffect, useMemo, useRef, useState } from "react";
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
import type { NewTaskPreset } from "@/features/create-task";
import { isTyping, plural } from "@/shared/lib";
import { Icon } from "@/shared/ui";
import { MobileNav, TeamsSummary } from "@/widgets/sidebar";
import { TaskList } from "@/widgets/task-list";
import { FilterMenu, FilterOption } from "./FilterMenu.tsx";

/** «Задачи»: the project's tasks as a list, with search and filters by status, readiness, time and more. */
export function TasksPage({ onNew, searchRef }: { onNew: (preset?: NewTaskPreset) => void; searchRef: React.RefObject<HTMLInputElement | null> }) {
  const [sp, setSp] = useSearchParams();
  const [focused, setFocused] = useState<string | undefined>();
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
  const newTask = () => onNew(f.epic && f.epic !== "none" ? { epic: f.epic } : undefined);

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

  const toggle = <T,>(list: T[], v: T) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  const preset = PRESETS.find((p) => sameSet(p.statuses, f.statuses));
  const statusValue = preset ? preset.name.toLowerCase() : f.statuses.length === 1 ? STATUS_NAME[f.statuses[0]].toLowerCase() : `${f.statuses.length} из ${STATUS_ORDER.length}`;
  const whoName = f.who === "me" ? "я" : f.who === "none" ? "никто" : f.who ? (people.get(f.who) ?? f.who) : "все";
  const epicName = f.epic === "none" ? "без эпика" : f.epic ? f.epic : "любой";

  return (
    <main className="main">
      <header className="topbar">
        <h1>Задачи</h1>
        <span className="muted d-only" style={{ fontSize: 12 }}>
          {shown.length} {plural(shown.length, "задача", "задачи", "задач")}
        </span>
        <span className="grow" />
        <label className="search wide">
          <Icon.search />
          <input
            ref={searchRef}
            type="search"
            placeholder="Название, номер, метка или вопрос"
            aria-label="Поиск задач"
            value={f.q}
            onChange={(e) => set({ q: e.target.value })}
            onKeyDown={(e) => e.key === "Escape" && (set({ q: "" }), e.currentTarget.blur())}
          />
          <kbd className="d-only">/</kbd>
        </label>
        <button type="button" className="btn primary" onClick={newTask}>
          <Icon.plus size={13} />
          <span className="d-only">Новая задача</span>
        </button>
      </header>

      <MobileNav />

      <nav className="presets" aria-label="Быстрые наборы">
        {PRESETS.map((p) => {
          const n = countOf(p.statuses);
          const on = preset?.id === p.id;
          const cls = [on ? "on" : "", p.id === "decisions" && n ? "amber" : ""].filter(Boolean).join(" ");
          return (
            <button key={p.id} type="button" className={cls} aria-pressed={on} onClick={() => set({ statuses: p.statuses })}>
              {p.name}
              <span className="n">{n || ""}</span>
            </button>
          );
        })}
        <span className="grow" />
        <label className="fsel d-only">
          Порядок
          <select value={f.sort} onChange={(e) => set({ sort: e.target.value as TaskSort })}>
            {(Object.keys(SORT_NAME) as TaskSort[]).map((s) => (
              <option key={s} value={s}>
                {SORT_NAME[s]}
              </option>
            ))}
          </select>
        </label>
        <label className="fsel d-only">
          Группы
          <select value={f.grouped ? "status" : "none"} onChange={(e) => set({ grouped: e.target.value === "status" })}>
            <option value="status">по статусу</option>
            <option value="none">без групп</option>
          </select>
        </label>
      </nav>

      <div className="filters" role="toolbar" aria-label="Фильтры">
        <FilterMenu label="Статус" value={statusValue} active={preset?.id !== "open"}>
          {STATUS_ORDER.map((s) => (
            <FilterOption key={s} kind="checkbox" checked={f.statuses.includes(s)} onChange={() => set({ statuses: toggle(f.statuses, s) })} hint={countOf([s]) || ""}>
              <StatusIcon status={s} size={12} /> {STATUS_NAME[s]}
            </FilterOption>
          ))}
        </FilterMenu>
        <FilterMenu label="Готовность" value={f.ready ? READINESS_NAME[f.ready] : "любая"} active={!!f.ready}>
          <span className="fhead">По критериям приёмки</span>
          <FilterOption kind="radio" checked={!f.ready} onChange={() => set({ ready: undefined })}>
            Любая
          </FilterOption>
          {(Object.keys(READINESS_NAME) as Readiness[]).map((r) => (
            <FilterOption key={r} kind="radio" checked={f.ready === r} onChange={() => set({ ready: r })} hint={READINESS_HINT[r]}>
              {cap(READINESS_NAME[r])}
            </FilterOption>
          ))}
        </FilterMenu>
        <FilterMenu label={f.by === "created" ? "Созданы" : "Обновлены"} value={f.time ? TIME_NAME[f.time] : "за всё время"} active={!!f.time}>
          <div className="seg full" role="group" aria-label="По какой дате">
            <button type="button" className={f.by === "updated" ? "on" : ""} aria-pressed={f.by === "updated"} onClick={() => set({ by: "updated" })}>
              Обновлены
            </button>
            <button type="button" className={f.by === "created" ? "on" : ""} aria-pressed={f.by === "created"} onClick={() => set({ by: "created" })}>
              Созданы
            </button>
          </div>
          <FilterOption kind="radio" checked={!f.time} onChange={() => set({ time: undefined })}>
            За всё время
          </FilterOption>
          {(Object.keys(TIME_NAME) as TimeRange[]).map((r) => (
            <FilterOption key={r} kind="radio" checked={f.time === r} onChange={() => set({ time: r })}>
              {cap(TIME_NAME[r])}
            </FilterOption>
          ))}
        </FilterMenu>
        <FilterMenu label="Приоритет" value={f.priorities.length ? f.priorities.map((p) => PRIORITY_NAME[p].toLowerCase()).join(", ") : "любой"} active={f.priorities.length > 0}>
          {PRIORITY_NAME.map((name, p) => (
            <FilterOption key={p} kind="checkbox" checked={f.priorities.includes(p)} onChange={() => set({ priorities: toggle(f.priorities, p) })}>
              {name}
            </FilterOption>
          ))}
        </FilterMenu>
        <FilterMenu label="Эпик" value={epicName} active={!!f.epic}>
          <FilterOption kind="radio" checked={!f.epic} onChange={() => set({ epic: undefined })}>
            Любой
          </FilterOption>
          <FilterOption kind="radio" checked={f.epic === "none"} onChange={() => set({ epic: "none" })}>
            Без эпика
          </FilterOption>
          {epics.map((e) => (
            <FilterOption key={e.id} kind="radio" checked={f.epic === e.id} onChange={() => set({ epic: e.id })} hint={e.id}>
              {e.title}
            </FilterOption>
          ))}
        </FilterMenu>
        {login && (
          <FilterMenu label="Ответственный" value={whoName} active={!!f.who}>
            <FilterOption kind="radio" checked={!f.who} onChange={() => set({ who: undefined })}>
              Все
            </FilterOption>
            <FilterOption kind="radio" checked={f.who === "me"} onChange={() => set({ who: "me" })}>
              Я
            </FilterOption>
            <FilterOption kind="radio" checked={f.who === "none"} onChange={() => set({ who: "none" })}>
              Никто не назначен
            </FilterOption>
            {[...people]
              .filter(([l]) => l !== login)
              .map(([l, name]) => (
                <FilterOption key={l} kind="radio" checked={f.who === l} onChange={() => set({ who: l })}>
                  {name}
                </FilterOption>
              ))}
          </FilterMenu>
        )}
        {isFiltered(f) && (
          <button type="button" className="btn ghost sm" onClick={() => setSp(writeFilter(parseFilter(new URLSearchParams()), new URLSearchParams(selected ? { task: selected } : {})), { replace: true })}>
            Сбросить
          </button>
        )}
      </div>

      {tasksQ.isPending ? (
        <div className="empty">Загрузка…</div>
      ) : tasksQ.isError ? (
        <div className="empty">Не удалось загрузить задачи: {tasksQ.error.message}</div>
      ) : (
        <div className="scroll">
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

      <footer className="hints">
        <span>
          <kbd>J</kbd> <kbd>K</kbd> навигация
        </span>
        <span>
          <kbd>↵</kbd> открыть
        </span>
        <span>
          <kbd>/</kbd> поиск
        </span>
        <span>
          <kbd>G</kbd> <kbd>B</kbd> доска
        </span>
        <span>
          <kbd>⌘K</kbd> команды
        </span>
        <span className="grow" />
        <TeamsSummary teams={teams} />
      </footer>
    </main>
  );
}

const READINESS_HINT: Record<Readiness, string> = { none: "критерии не записаны", zero: "0 из N", partial: "часть выполнена", full: "N из N" };

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
