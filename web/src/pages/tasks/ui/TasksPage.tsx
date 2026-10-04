import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { personName, useMembers } from "@/entities/project";
import { useSession } from "@/entities/session";
import { EpicIcon, inTaskViews, inViewOf, isView, type ViewId, VIEWS, useEpicMap, useTasks } from "@/entities/task";
import { type Team, useTeamMap } from "@/entities/team";
import type { NewTaskPreset } from "@/features/create-task";
import { isTyping, type Layout, plural, readPref, writePref } from "@/shared/lib";
import { Icon } from "@/shared/ui";
import { Board } from "@/widgets/board";
import { TaskList } from "@/widgets/task-list";

export function TasksPage({ onNew, searchRef }: { onNew: (preset?: NewTaskPreset) => void; searchRef: React.RefObject<HTMLInputElement | null> }) {
  const params = useParams();
  const view: ViewId = isView(params.view) ? params.view : "active";
  const [sp, setSp] = useSearchParams();
  const navigate = useNavigate();
  const layout: Layout = (sp.get("layout") as Layout | null) ?? readPref<Layout>("genie.layout", "list");
  const [query, setQuery] = useState("");
  const [showDone, setShowDone] = useState(() => readPref<string>("genie.showDone", "0") === "1");
  const [focused, setFocused] = useState<string | undefined>();
  const tasksQ = useTasks();
  const teams = useTeamMap();
  const session = useSession().data;
  const login = session?.mode === "users" ? session.user.login : undefined;
  const members = useMembers(login ? session?.project : undefined).data;
  const people = useMemo(() => new Map((members ?? []).map((m) => [m.user.login, personName(m.user)])), [members]);
  const selected = sp.get("task") ?? undefined;
  const epicFilter = sp.get("epic") ?? undefined;
  const epic = useEpicMap().get(epicFilter ?? "");
  const newTask = () => onNew(epicFilter ? { epic: epicFilter } : undefined);
  const clearEpic = () => {
    const next = new URLSearchParams(sp);
    next.delete("epic");
    setSp(next);
  };

  const q = query.trim().toLowerCase();
  const visible = (tasksQ.data ?? []).filter((t) => (epicFilter ? t.parent === epicFilter : inTaskViews(t)));
  const matches = visible.filter((t) => !q || t.title.toLowerCase().includes(q) || t.id.toLowerCase().includes(q) || t.labels.some((l) => l.includes(q)));
  const inView = matches.filter((t) => inViewOf(t, view, login));
  const boardTasks = VIEWS[view].mine ? matches.filter((t) => t.assignee === login) : matches;
  const ordered = useMemo(() => inView, [inView]);

  const setLayout = (l: Layout) => {
    writePref("genie.layout", l);
    const next = new URLSearchParams(sp);
    next.set("layout", l);
    setSp(next);
  };
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
      if (layout !== "list" || isTyping(e)) return;
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
      } else if (e.key === "b") {
        setLayout(layout === "list" ? "board" : "list");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <main className="main">
      <header className="topbar">
        <h1>{layout === "board" ? "Доска" : VIEWS[view].name}</h1>
        {epicFilter && (
          <span className="filter-chip" title={epic ? `Эпик ${epic.id}: ${epic.title}` : undefined}>
            <EpicIcon size={11} />
            <Link to={`/epic/${encodeURIComponent(epicFilter)}`}>Эпик {epicFilter}</Link>
            <button type="button" onClick={clearEpic} aria-label="Снять фильтр по эпику">
              <Icon.close size={10} />
            </button>
          </span>
        )}
        <span className="muted d-only" style={{ fontSize: 12 }}>
          {(layout === "board" ? boardTasks : inView).length} {plural((layout === "board" ? boardTasks : inView).length, "задача", "задачи", "задач")}
        </span>
        <div className="seg icons" role="group" aria-label="Представление">
          <button type="button" className={layout === "list" ? "on" : ""} aria-pressed={layout === "list"} aria-label="Список" title="Список" onClick={() => setLayout("list")}>
            <Icon.list size={13} />
          </button>
          <button type="button" className={layout === "board" ? "on" : ""} aria-pressed={layout === "board"} aria-label="Доска" title="Доска" onClick={() => setLayout("board")}>
            <Icon.board size={13} />
          </button>
        </div>
        <span className="grow" />
        <label className="search">
          <Icon.search />
          <input ref={searchRef} type="search" placeholder="Поиск задач" aria-label="Поиск задач" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && (setQuery(""), e.currentTarget.blur())} />
          <kbd className="d-only">/</kbd>
        </label>
        {layout === "board" && (
          <button
            type="button"
            className="btn d-only"
            aria-pressed={showDone}
            onClick={() => {
              writePref("genie.showDone", showDone ? "0" : "1");
              setShowDone(!showDone);
            }}
          >
            {showDone ? "Скрыть завершённые" : "Показать завершённые"}
          </button>
        )}
        <button type="button" className="btn primary" onClick={newTask}>
          <Icon.plus size={13} />
          <span className="d-only">Новая задача</span>
        </button>
      </header>

      {layout === "list" && (
        <div className="m-only m-nav" role="group" aria-label="Разделы">
          {(Object.keys(VIEWS) as ViewId[])
            .filter((v) => login || !VIEWS[v].mine)
            .map((v) => {
              const n = visible.filter((t) => inViewOf(t, v, login) && (!VIEWS[v].mine || t.status !== "done")).length;
              const cls = v === view ? "on" : v === "decisions" && n ? "amber" : "";
              return (
                <button key={v} type="button" className={cls} onClick={() => navigate({ pathname: `/${v}`, search: sp.toString() })}>
                  {VIEWS[v].name} {n || ""}
                </button>
              );
            })}
          <button type="button" onClick={() => navigate("/epics")}>
            Эпики
          </button>
          {[...teams.values()]
            .filter((t) => t.state === "active")
            .map((t) => (
              <button key={t.id} type="button" onClick={() => navigate(`/team/${encodeURIComponent(t.id)}`)}>
                Команда {t.id}
              </button>
            ))}
        </div>
      )}

      {tasksQ.isPending ? (
        <div className="empty">Загрузка…</div>
      ) : tasksQ.isError ? (
        <div className="empty">Не удалось загрузить задачи: {tasksQ.error.message}</div>
      ) : layout === "board" ? (
        <Board tasks={boardTasks} teams={teams} selected={selected} showDone={showDone} onShowDone={() => setShowDone(true)} onOpen={open} />
      ) : (
        <div className="scroll">
          <TaskList tasks={ordered} statuses={VIEWS[view].statuses} teams={teams} people={people} focused={focused} selected={selected} onOpen={open} onFocus={setFocused} />
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
          <kbd>B</kbd> список / доска
        </span>
        <span>
          <kbd>C</kbd> новая
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

function TeamsSummary({ teams }: { teams: Map<string, Team> }) {
  const active = [...teams.values()].filter((t) => t.state === "active");
  const agents = active.reduce((n, t) => n + t.members.filter((m) => m.activity === "working").length, 0);
  if (!active.length) return <span>Нет активных команд</span>;
  return (
    <span>
      {active.length} {plural(active.length, "команда", "команды", "команд")} · {agents} {plural(agents, "агент работает", "агента работают", "агентов работают")}
    </span>
  );
}
