import { useState } from "react";
import { useSearchParams } from "react-router";
import { useSession } from "@/entities/session";
import { EpicIcon, inTaskViews, OPEN, useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import type { NewTaskPreset } from "@/features/create-task";
import { plural, readPref, writePref } from "@/shared/lib";
import { Icon } from "@/shared/ui";
import { Board } from "@/widgets/board";
import { MobileNav, TeamsSummary } from "@/widgets/sidebar";

/** «Доска»: the project's Kanban board, narrowed by filters («Мои задачи», an epic). */
export function BoardPage({ onNew, searchRef }: { onNew: (preset?: NewTaskPreset) => void; searchRef: React.RefObject<HTMLInputElement | null> }) {
  const [sp, setSp] = useSearchParams();
  const [query, setQuery] = useState("");
  const [showDone, setShowDone] = useState(() => readPref<string>("genie.showDone", "0") === "1");
  const [mineOnly, setMineOnly] = useState(() => readPref<string>("genie.board.mine", "0") === "1");
  const tasksQ = useTasks();
  const teams = useTeamMap();
  const session = useSession().data;
  const login = session?.mode === "users" ? session.user.login : undefined;
  const epicFilter = sp.get("epic") ?? undefined;
  const mine = !!login && mineOnly;

  const all = tasksQ.data ?? [];
  const epics = all.filter((t) => t.type === "epic" && (OPEN.includes(t.status) || t.id === epicFilter));
  const q = query.trim().toLowerCase();
  const tasks = all.filter(
    (t) =>
      (epicFilter ? t.parent === epicFilter : inTaskViews(t)) &&
      (!mine || t.assignee === login) &&
      (!q || t.title.toLowerCase().includes(q) || t.id.toLowerCase().includes(q) || t.labels.some((l) => l.includes(q))),
  );
  const openCount = tasks.filter((t) => OPEN.includes(t.status)).length;
  const myOpen = login ? all.filter((t) => inTaskViews(t) && t.assignee === login && OPEN.includes(t.status)).length : 0;

  const setParam = (key: string, value?: string) => {
    const next = new URLSearchParams(sp);
    if (value) next.set(key, value);
    else next.delete(key);
    setSp(next);
  };
  const toggleMine = () => {
    writePref("genie.board.mine", mineOnly ? "0" : "1");
    setMineOnly(!mineOnly);
  };

  return (
    <main className="main">
      <header className="topbar">
        <h1>Доска</h1>
        <span className="muted d-only" style={{ fontSize: 12 }}>
          {openCount} {plural(openCount, "задача", "задачи", "задач")}
        </span>
        <span className="grow" />
        <label className="search">
          <Icon.search />
          <input ref={searchRef} type="search" placeholder="Поиск на доске" aria-label="Поиск на доске" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && (setQuery(""), e.currentTarget.blur())} />
          <kbd className="d-only">/</kbd>
        </label>
        <button type="button" className="btn primary" onClick={() => onNew(epicFilter ? { epic: epicFilter } : undefined)}>
          <Icon.plus size={13} />
          <span className="d-only">Новая задача</span>
        </button>
      </header>

      <MobileNav />

      <div className="filters" role="toolbar" aria-label="Фильтры доски">
        {login && (
          <button type="button" className={`fbtn${mine ? " on" : ""}`} aria-pressed={mine} onClick={toggleMine}>
            <Icon.user size={13} />
            Мои задачи
            <span className="k">{myOpen || ""}</span>
          </button>
        )}
        <label className={`fbtn select${epicFilter ? " on" : ""}`}>
          <EpicIcon size={12} />
          <span className="k">Эпик</span>
          <select value={epicFilter ?? ""} onChange={(e) => setParam("epic", e.target.value || undefined)} aria-label="Эпик">
            <option value="">любой</option>
            {epics.map((e) => (
              <option key={e.id} value={e.id}>
                {e.id} · {e.title}
              </option>
            ))}
          </select>
        </label>
        <span className="grow" />
        <span className="muted d-only" style={{ fontSize: 12 }}>
          Перетащите карточку, чтобы сменить статус
        </span>
        <button
          type="button"
          className="fbtn d-only"
          aria-pressed={showDone}
          onClick={() => {
            writePref("genie.showDone", showDone ? "0" : "1");
            setShowDone(!showDone);
          }}
        >
          {showDone ? "Скрыть завершённые" : "Показать завершённые"}
        </button>
      </div>

      {tasksQ.isPending ? (
        <div className="empty">Загрузка…</div>
      ) : tasksQ.isError ? (
        <div className="empty">Не удалось загрузить задачи: {tasksQ.error.message}</div>
      ) : (
        <Board tasks={tasks} teams={teams} showDone={showDone} onShowDone={() => setShowDone(true)} onOpen={(id) => setParam("task", id)} />
      )}

      <footer className="hints">
        <span>
          <kbd>G</kbd> <kbd>T</kbd> список задач
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
