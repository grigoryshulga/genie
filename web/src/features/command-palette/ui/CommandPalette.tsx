import { type ReactNode, useEffect, useMemo, useState } from "react";
import { DocFileIcon, DocStatusBadge, snippetParts, useDebounced, useDocSearch } from "@/entities/doc";
import { useSession } from "@/entities/session";
import { EpicIcon, type PresetId, PRESETS, StatusIcon, useTasks } from "@/entities/task";
import { useTeams } from "@/entities/team";
import { Icon, Modal } from "@/shared/ui";

export interface PaletteActions {
  newTask: () => void;
  /** «Новая задача» in the «Обсудить с агентом» mode. */
  newIdea: () => void;
  go: (where: "board" | "tasks" | "epics") => void;
  /** The task list with a preset's statuses; `mine` keeps the viewer's tasks. */
  preset: (id: PresetId, mine?: boolean) => void;
  openTask: (id: string) => void;
  openTeam: (id: string) => void;
  openDoc: (path: string) => void;
  openDocs: () => void;
  newDoc: (kind: "page" | "note", title?: string) => void;
  searchDocs: (q: string) => void;
}

interface Item {
  key: string;
  icon: ReactNode;
  label: string;
  hint?: string;
  /** Second line of a docs hit (a highlighted snippet). */
  sub?: ReactNode;
  /** Rendered next to the label (docs status). */
  mark?: ReactNode;
  group: "ДЕЙСТВИЯ" | "ПЕРЕЙТИ" | "КОМАНДЫ" | "ЗАДАЧИ" | "ДОКУМЕНТАЦИЯ";
  run: () => void;
}

const GROUP_ORDER: Record<Item["group"], number> = { ЗАДАЧИ: 0, ДОКУМЕНТАЦИЯ: 1, ДЕЙСТВИЯ: 2, ПЕРЕЙТИ: 3, КОМАНДЫ: 4 };

export function CommandPalette({ onClose, actions }: { onClose: () => void; actions: PaletteActions }) {
  const tasks = useTasks().data ?? [];
  const teams = useTeams().data ?? [];
  // "My tasks" needs users: in the local mode nobody is responsible for anything.
  const mine = useSession().data?.mode === "users";
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);
  const [docsOnly, setDocsOnly] = useState(false);
  const query = q.trim();
  const debounced = useDebounced(q, 180);
  const docs = useDocSearch(debounced.trim(), debounced.trim().length > 0);

  const items = useMemo<Item[]>(() => {
    const base: Item[] = [
      { key: "new", icon: <Icon.plus />, label: "Новая задача", hint: "C", group: "ДЕЙСТВИЯ", run: actions.newTask },
      { key: "idea", icon: <Icon.plus />, label: "Обсудить идею с агентом", group: "ДЕЙСТВИЯ", run: actions.newIdea },
      { key: "go-board", icon: <Icon.board />, label: "Доска", hint: "G B", group: "ПЕРЕЙТИ", run: () => actions.go("board") },
      { key: "go-tasks", icon: <Icon.list />, label: "Задачи", hint: "G T", group: "ПЕРЕЙТИ", run: () => actions.go("tasks") },
      { key: "go-epics", icon: <EpicIcon />, label: "Эпики", hint: "G E", group: "ПЕРЕЙТИ", run: () => actions.go("epics") },
      ...(mine ? [{ key: "p-mine", icon: <Icon.user />, label: "Задачи: мои", group: "ПЕРЕЙТИ" as const, run: () => actions.preset("open", true) }] : []),
      ...PRESETS.filter((p) => p.id !== "open").map((p) => ({ key: `p-${p.id}`, icon: <StatusIcon status={p.statuses[0]} />, label: `Задачи: ${p.name.toLowerCase()}`, group: "ПЕРЕЙТИ" as const, run: () => actions.preset(p.id) })),
      { key: "docs", icon: <DocFileIcon />, label: "Документация", hint: "docs/", group: "ПЕРЕЙТИ", run: actions.openDocs },
      ...teams.filter((t) => t.state === "active").map((t) => ({ key: `t-${t.id}`, icon: <span className="spin" />, label: `Команда ${t.id}`, hint: t.taskInfo?.title, group: "КОМАНДЫ" as const, run: () => actions.openTeam(t.id) })),
      ...tasks.map((t) => ({ key: t.id, icon: <StatusIcon status={t.status} />, label: `${t.id}  ${t.title}`, hint: t.labels.join(", "), group: "ЗАДАЧИ" as const, run: () => actions.openTask(t.id) })),
    ];

    const docsHits: Item[] = (docs.data?.results ?? []).slice(0, 6).map((result) => ({
      key: `d-${result.path}`,
      icon: <DocFileIcon />,
      label: result.title,
      hint: result.path,
      mark: <DocStatusBadge status={result.status} />,
      sub: (
        <span className="pal-snippet">
          {snippetParts(result.snippet)
            .slice(0, 12)
            .map((part, i) => (part.hit ? <mark key={i}>{part.text}</mark> : <span key={i}>{part.text}</span>))}
        </span>
      ),
      group: "ДОКУМЕНТАЦИЯ",
      run: () => actions.openDoc(result.path),
    }));
    if (query) {
      if (!docsHits.length) {
        docsHits.push({
          key: "d-search",
          icon: <Icon.search />,
          label: `Найти в документации «${query}»`,
          group: "ДОКУМЕНТАЦИЯ",
          run: () => actions.searchDocs(query),
        });
      }
      docsHits.push({
        key: "d-note",
        icon: <Icon.plus />,
        label: `Быстрая заметка «${query}» в inbox/`,
        group: "ДЕЙСТВИЯ",
        run: () => actions.newDoc("note", query),
      });
      docsHits.push({
        key: "d-page",
        icon: <Icon.plus />,
        label: `Новая страница «${query}»`,
        group: "ДЕЙСТВИЯ",
        run: () => actions.newDoc("page", query),
      });
    }

    const all = [...base, ...docsHits];
    const s = query.toLowerCase();
    // Docs hits come from the server already ranked and matched (title, summary,
    // headings, body, tags, aliases). Re-filtering them by substring would drop
    // e.g. alias-only matches, so they only obey the `docsOnly` scope.
    const isDocItem = (item: Item) => item.group === "ДОКУМЕНТАЦИЯ" || item.key === "d-note" || item.key === "d-page";
    const matched = s ? all.filter((i) => isDocItem(i) || `${i.label} ${i.hint ?? ""}`.toLowerCase().includes(s)) : all;
    const scoped = docsOnly ? matched.filter(isDocItem) : matched;
    return scoped.sort((a, b) => GROUP_ORDER[a.group] - GROUP_ORDER[b.group]).slice(0, 60);
  }, [query, tasks, teams, actions, docs.data, docsOnly, mine]);

  useEffect(() => setIdx(0), [q, docsOnly]);

  const run = (item: Item | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  let lastGroup: string | undefined;

  return (
    <Modal label="Команды" onClose={onClose}>
      <div className="palette">
        <input
          autoFocus
          aria-label="Поиск команд, задач и документации"
          placeholder="Задача, команда, документация или действие…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Tab") {
              e.preventDefault();
              setDocsOnly((v) => !v);
              return;
            }
            if (e.key === "ArrowDown") setIdx((i) => Math.min(items.length - 1, i + 1));
            else if (e.key === "ArrowUp") setIdx((i) => Math.max(0, i - 1));
            else if (e.key === "Enter") run(items[idx]);
            else return;
            e.preventDefault();
          }}
        />
        <div className="items" role="listbox">
          {items.map((i, n) => {
            const header = i.group !== lastGroup ? i.group : undefined;
            lastGroup = i.group;
            return (
              <div key={i.key}>
                {header && (
                  <div className={`pal-group${docsOnly ? " on" : ""}`}>
                    {header}
                    {header === "ДОКУМЕНТАЦИЯ" && docsOnly ? <span className="muted"> · только документация</span> : null}
                  </div>
                )}
                <button type="button" role="option" aria-selected={n === idx} className={`item${n === idx ? " on" : ""}`} onMouseEnter={() => setIdx(n)} onClick={() => run(i)}>
                  {i.icon}
                  <span className="pal-col">
                    <span className="pal-label">
                      {i.label}
                      {i.mark}
                    </span>
                    {i.sub && <span className="pal-sub">{i.sub}</span>}
                  </span>
                  {i.hint && <span className="hint">{i.hint}</span>}
                </button>
              </div>
            );
          })}
          {!items.length && (
            <div className="empty" style={{ padding: 30 }}>
              {docsOnly && !docs.isPending ? "В документации ничего не найдено" : "Ничего не найдено"}
            </div>
          )}
        </div>
        <div className="pal-foot muted">
          <kbd>↑↓</kbd> выбрать <kbd>↵</kbd> открыть <kbd>Tab</kbd> только документация <kbd>Esc</kbd> закрыть
        </div>
      </div>
    </Modal>
  );
}
