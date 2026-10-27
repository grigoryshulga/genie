import { useState } from "react";
import { useSearchParams } from "react-router";
import { useSession } from "@/entities/session";
import { inTaskViews, OPEN, useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import { useOnline } from "@/shared/api";
import { readPref, writePref } from "@/shared/lib";
import { Banner, Button, EmptyState, ErrorState, FilterOption, Icon, Kbd, PageHeader, Popover, Skeleton } from "@/shared/ui";
import { Board } from "@/widgets/board";
import { TeamsSummary } from "@/widgets/topbar";

/**
 * The board offers the same "new task" action as the C shortcut, so one code path opens the dialog.
 * The shortcut lives in the app shell, where it also serves every other screen.
 */
const pressC = () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "c", bubbles: true }));

/** «Доска»: the project's six lanes, narrowed by filters («Мои задачи», an epic). */
export function BoardPage() {
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
  const myOpen = login ? all.filter((t) => inTaskViews(t) && t.assignee === login && OPEN.includes(t.status)).length : 0;
  const activeFilters = (mine ? 1 : 0) + (epicFilter ? 1 : 0);

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
      <PageHeader
        title="Доска"
        toolbar={
          <>
            <Popover label="Фильтры" icon={<Icon.filter size={13} />} count={activeFilters}>
              {login && (
                <section className="fgroup">
                  <h3 className="fhead">Исполнитель</h3>
                  <FilterOption kind="checkbox" checked={mine} onChange={toggleMine} hint={myOpen > 0 ? myOpen : undefined}>
                    Мои задачи
                  </FilterOption>
                </section>
              )}
              <section className="fgroup">
                <h3 className="fhead">Эпик</h3>
                <FilterOption kind="radio" checked={!epicFilter} onChange={() => setParam("epic", undefined)}>
                  Любой
                </FilterOption>
                {epics.map((e) => (
                  <FilterOption key={e.id} kind="radio" checked={epicFilter === e.id} onChange={() => setParam("epic", e.id)} hint={e.id}>
                    {e.title}
                  </FilterOption>
                ))}
              </section>
            </Popover>
          </>
        }
      />
      {!online && (
        <Banner tone="signal" title="Нет связи с сервером">
          Перетаскивать карточки нельзя, пока соединение не вернётся. Доска обновится сама.
        </Banner>
      )}

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
        <Board tasks={tasks} teams={teams} selected={sp.get("task") ?? undefined} showDone={showDone} onDoneVisibilityChange={setDoneVisibility} onOpen={(id) => setParam("task", id)} offline={!online} />
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
