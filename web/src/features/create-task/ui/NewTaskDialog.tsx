import { useState } from "react";
import { useMeta } from "@/entities/project";
import { EpicIcon, PRIORITY_NAME, PriorityIcon, StatusIcon, useCreateTask, useEpicMap } from "@/entities/task";
import { useStartIdea } from "@/features/shape-idea";
import type { TaskType } from "@/shared/api";
import { Icon, Modal, ResizableTextarea, useToast } from "@/shared/ui";
import "@/features/shape-idea/ui/idea.css";

const TYPES: [TaskType, string][] = [
  ["task", "Задача"],
  ["bug", "Баг"],
  ["spike", "Исследование"],
  ["epic", "Эпик"],
];

export interface NewTaskPreset {
  type?: TaskType;
  epic?: string;
  /** Open in the «Обсудить с менеджером» mode. */
  idea?: boolean;
}

export function NewTaskDialog({
  preset,
  onClose,
  onCreated,
  onIdea,
}: {
  preset?: NewTaskPreset;
  onClose: () => void;
  onCreated: (id: string, type: string) => void;
  /** An idea went to a planner: open the chat with it. */
  onIdea: (team: string, member: string) => void;
}) {
  const meta = useMeta().data;
  const create = useCreateTask();
  const toast = useToast();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [criteria, setCriteria] = useState("");
  const [priority, setPriority] = useState(2);
  const [type, setType] = useState<TaskType>(preset?.type ?? "task");
  const [epic, setEpic] = useState(preset?.epic ?? "");
  const epics = [...useEpicMap().values()].filter((e) => e.status !== "done" && e.status !== "cancelled");
  const isEpic = type === "epic";
  const [labels, setLabels] = useState("");
  // Ideas are filed on their own, not inside an epic.
  const canTalk = !preset?.epic;
  const [talk, setTalk] = useState(!!preset?.idea && canTalk);
  const [idea, setIdea] = useState("");
  const start = useStartIdea();

  const startIdea = () => {
    if (!idea.trim() || start.isPending) return;
    start.mutate(
      { text: idea.trim() },
      {
        onSuccess: (r) => {
          toast(`Идея ${r.task} у планировщика · оркестратор подождёт плана`);
          onIdea(r.team, r.member);
        },
        onError: (e) => toast(`Разговор не начат: ${e.message}`, "error"),
      },
    );
  };

  const submit = () => {
    if (talk) return startIdea();
    if (!title.trim()) return;
    create.mutate(
      {
        title: title.trim(),
        description: description.trim() || undefined,
        acceptance: criteria.split("\n").map((s) => s.trim()).filter(Boolean),
        priority,
        type,
        labels: labels.split(",").map((s) => s.trim()).filter(Boolean),
        parent: !isEpic && epic ? epic : undefined,
      },
      {
        onSuccess: (t) => {
          toast(`${isEpic ? "Эпик" : "Задача"} ${t.id} во входящих · оркестратор уведомлён`);
          onCreated(t.id, t.type);
        },
        onError: (e) => toast(`Не создано: ${e.message}`, "error"),
      },
    );
  };

  return (
    <Modal label={talk ? "Обсудить с менеджером проекта" : isEpic ? "Новый эпик" : "Новая задача"} onClose={onClose} className="nt-dialog">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && submit()}
        className="dlg-form"
      >
        <div className="mh">
          <span className="g-tag">{meta?.project ?? "genie"}</span>
          <Icon.chevron size={12} />
          <span className="nt-title">{talk ? "Обсудить с менеджером проекта" : isEpic ? "Новый эпик во входящие" : "Новая задача во входящие"}</span>
          <span className="grow" />
          {canTalk && (
            <div role="radiogroup" aria-label="Как завести" className="nt-mode">
              <button type="button" role="radio" aria-checked={!talk} onClick={() => setTalk(false)}>
                Заполнить самому
              </button>
              <button type="button" role="radio" aria-checked={talk} onClick={() => setTalk(true)}>
                <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                  <path d="M2.5 4.5a2 2 0 0 1 2-2h7a2 2 0 0 1 2 2v4.5a2 2 0 0 1-2 2H7l-3 2.5v-2.5a2 2 0 0 1-1.5-2z" />
                </svg>
                Обсудить с менеджером
              </button>
            </div>
          )}
          <button type="button" className="g-icon-btn" onClick={onClose} aria-label="Закрыть">
            <Icon.close />
          </button>
        </div>
        {talk ? (
          <div className="mb">
            <label className="g-field">
              <span className="g-label">Идея как есть</span>
              <ResizableTextarea
                autoFocus
                rows={7}
                required
                value={idea}
                onChange={(e) => setIdea(e.target.value)}
                placeholder="Что хочется получить и зачем. Можно сумбурно: планировщик сам спросит, чего не хватает."
              />
            </label>
            <ol className="nt-steps">
              <li>Планировщик задаст вопросы: цель, для кого, рамки, что уже есть</li>
              <li>Предложит план: одну задачу или эпик с задачами и критериями</li>
              <li>Вы правите план и одной кнопкой заводите всё во входящие</li>
            </ol>
          </div>
        ) : (
          <div className="mb">
            <label className="g-field">
              <span className="g-label">Название</span>
              <input className="g-input" autoFocus required value={title} onChange={(e) => setTitle(e.target.value)} placeholder={isEpic ? "Крупная веха: что должно получиться?" : "Что нужно сделать?"} />
            </label>
            <label className="g-field">
              <span className="g-label">{isEpic ? "Цель" : "Описание"}</span>
              <ResizableTextarea
                rows={4}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder={isEpic ? "Зачем эта веха, что входит и что нет. Поддерживается markdown." : "Контекст, ограничения, ссылки. Поддерживается markdown."}
              />
            </label>
            <label className="g-field">
              <span className="g-label">{isEpic ? "Критерии успеха — по одному в строке (можно оставить оркестратору)" : "Критерии приёмки — по одному в строке (можно оставить оркестратору)"}</span>
              <ResizableTextarea rows={2} value={criteria} onChange={(e) => setCriteria(e.target.value)} />
            </label>
            <div className="opts">
              <label className="nt-opt">
                <PriorityIcon priority={priority} size={13} />
                <select aria-label="Приоритет" value={priority} onChange={(e) => setPriority(Number(e.target.value))}>
                  {PRIORITY_NAME.map((p, i) => (
                    <option key={p} value={i}>
                      {p}
                    </option>
                  ))}
                </select>
              </label>
              <select aria-label="Тип" value={type} onChange={(e) => setType(e.target.value as TaskType)}>
                {TYPES.map(([v, n]) => (
                  <option key={v} value={v}>
                    {n}
                  </option>
                ))}
              </select>
              {!isEpic && epics.length > 0 && (
                <label className="nt-opt">
                  <EpicIcon size={13} />
                  <select aria-label="Эпик" value={epic} onChange={(e) => setEpic(e.target.value)}>
                    <option value="">Без эпика</option>
                    {epics.map((e) => (
                      <option key={e.id} value={e.id}>
                        {e.id} · {e.title}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <input aria-label="Метки" placeholder="метки через запятую" value={labels} onChange={(e) => setLabels(e.target.value)} className="nt-labels" />
            </div>
          </div>
        )}
        <div className="mf">
          <StatusIcon status={talk ? "refining" : "inbox"} />
          {talk
            ? "Менеджер проекта поможет подготовить план до запуска задач"
            : isEpic
              ? "Менеджер проекта уточнит цель и выделит задачи эпика"
              : "Менеджер проекта получит задачу и уточнит детали"}
          <span className="grow" />
          <button type="button" className="g-btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className="g-btn primary" disabled={talk ? !idea.trim() || start.isPending : !title.trim() || create.isPending}>
            {talk ? "Начать разговор" : "Создать"} <span className="nt-kbd">⌘↵</span>
          </button>
        </div>
      </form>
    </Modal>
  );
}
