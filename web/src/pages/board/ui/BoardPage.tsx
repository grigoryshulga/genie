import { useState } from "react";
import { useSearchParams } from "react-router";
import { useSession } from "@/entities/session";
import { EpicIcon, inTaskViews, OPEN, useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import { useOnline } from "@/shared/api";
import { plural, readPref, writePref } from "@/shared/lib";
import { Banner, Button, Character, Chip, EmptyState, ErrorState, Icon, Kbd, Skeleton, characterState, cx } from "@/shared/ui";
import { Board } from "@/widgets/board";
import { ContextSearch, TeamsSummary } from "@/widgets/topbar";
import { boardHeadline, boardStrip, type BoardPulse } from "../model.ts";
import { Arcs } from "./Arcs.tsx";

/**
 * The board offers the same "new task" action as the C shortcut, so one code path opens the dialog.
 * The shortcut lives in the app shell, where it also serves every other screen.
 */
const pressC = () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "c", bubbles: true }));

const weekday = () => new Date().toLocaleDateString("ru-RU", { weekday: "long" });

/** «Доска»: the project's state in one headline, then the six lanes, narrowed by filters («Мои задачи», an epic). */
export function BoardPage({ searchRef }: { searchRef: React.RefObject<HTMLInputElement | null> }) {
  const [sp, setSp] = useSearchParams();
  const query = sp.get("q") ?? "";
  const [showDone, setShowDone] = useState(() => readPref<string>("genie.showDone", "0") === "1");
  const [mineOnly, setMineOnly] = useState(() => readPref<string>("genie.board.mine", "0") === "1");
  const tasksQ = useTasks();
  const teams = useTeamMap();
  const online = useOnline();
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

  // The headline and the strip state the project, not the filter: they are what is really going on.
  const view = all.filter(inTaskViews);
  const waiting = view.filter((t) => t.status === "needs_owner");
  const pulse: BoardPulse = {
    waiting: waiting.length,
    working: [...teams.values()].filter((t) => t.state === "active" && t.members.some((m) => m.activity === "working")).length,
    review: view.filter((t) => t.status === "review").length,
    open: view.filter((t) => OPEN.includes(t.status)).length,
  };
  const state = characterState({ working: pulse.working, waiting: pulse.waiting });
  const head = boardHeadline(pulse);
  const first = waiting[0];
  const strip = boardStrip(state, pulse, first && { id: first.id, title: first.title });
  const project = session?.projects.find((p) => p.slug === session.project)?.name ?? session?.project;

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
  const setDoneVisibility = (visible: boolean) => {
    writePref("genie.showDone", visible ? "1" : "0");
    setShowDone(visible);
  };

  return (
    <main className="main g-board-page" aria-label="Доска">
      <header className="g-board-hero">
        <Arcs className="g-board-arcs" />
        <p className="g-eyebrow">
          {project ? `${project} · ` : ""}
          {weekday()}
        </p>
        <h1 className="g-h1 g-board-h1">
          {head.title}
          {head.stop && <span className="g-stop">.</span>}
        </h1>
        <p className="g-board-lead">{head.lead}</p>
      </header>

      <section className="g-board-pulse" aria-label="Что происходит сейчас">
        <Character state={state} size={64} />
        <p className="g-pulse">
          <strong>{strip.say}</strong>
          <span>{strip.sub}</span>
        </p>
        <dl className="g-pulse-nums">
          <div>
            <dt>Ждут решения</dt>
            <dd>{pulse.waiting}</dd>
          </div>
          <div>
            <dt>Команды в работе</dt>
            <dd>{pulse.working}</dd>
          </div>
          <div>
            <dt>На ревью</dt>
            <dd>{pulse.review}</dd>
          </div>
        </dl>
      </section>

      {!online && (
        <Banner tone="signal" title="Нет связи с сервером">
          Перетаскивать карточки нельзя, пока соединение не вернётся. Доска обновится сама.
        </Banner>
      )}

      <div className="g-toolbar" role="toolbar" aria-label="Фильтры доски">
        {login && (
          <Chip pressed={mine} onClick={toggleMine}>
            <Icon.user size={13} />
            Мои задачи
            {myOpen > 0 && <span className="k g-num">{myOpen}</span>}
          </Chip>
        )}
        <label className={cx("g-chip g-chip-select", epicFilter && "on")}>
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
        <Chip pressed={showDone} onClick={() => setDoneVisibility(!showDone)}>
          Готово
        </Chip>
        <span className="grow" />
        <span className="g-toolbar-count g-num">
          {openCount} {plural(openCount, "задача", "задачи", "задач")}
        </span>
        <ContextSearch inputRef={searchRef} className="mobile-context-search" />
      </div>

      {tasksQ.isPending ? (
        <div className="g-board g-board-skeleton" aria-busy="true" aria-label="Загрузка доски">
          {[0, 1, 2, 3].map((lane) => (
            <div key={lane} className="g-lane">
              <Skeleton width={120} height={14} />
              {[0, 1, 2].map((card) => (
                <Skeleton key={card} height={104} />
              ))}
            </div>
          ))}
        </div>
      ) : tasksQ.isError ? (
        <div className="g-board-state">
          <ErrorState message={`Не удалось загрузить задачи: ${tasksQ.error.message}`} onRetry={() => void tasksQ.refetch()} />
        </div>
      ) : !tasks.length ? (
        <div className="g-board-state">
          {q || mine ? (
            <EmptyState
              title="Ничего не нашлось"
              action={
                <Button
                  variant="secondary"
                  onClick={() => {
                    writePref("genie.board.mine", "0");
                    setMineOnly(false);
                    perReset(setSp, sp);
                  }}
                >
                  Сбросить фильтры
                </Button>
              }
            >
              По этому поиску и фильтрам на доске нет задач.
            </EmptyState>
          ) : (
            <EmptyState
              title="Здесь тихо"
              action={
                <Button variant="primary" onClick={pressC}>
                  Создать задачу
                </Button>
              }
            >
              Открытых задач нет. Создайте новую, и Genie возьмётся за дело.
            </EmptyState>
          )}
        </div>
      ) : (
        <Board tasks={tasks} teams={teams} showDone={showDone} onDoneVisibilityChange={setDoneVisibility} onOpen={(id) => setParam("task", id)} offline={!online} />
      )}

      <footer className="g-hints" aria-label="Подсказки клавиш">
        <span>
          <Kbd>G</Kbd> <Kbd>T</Kbd> список задач
        </span>
        <span>
          <Kbd>C</Kbd> новая
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

/** Drops the query and the epic, keeping the open task in the address. */
function perReset(setSp: ReturnType<typeof useSearchParams>[1], sp: URLSearchParams) {
  const next = new URLSearchParams(sp);
  next.delete("q");
  next.delete("epic");
  setSp(next, { replace: true });
}
