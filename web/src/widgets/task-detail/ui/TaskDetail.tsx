import { TaskIntelligence } from "./TaskIntelligence.tsx";
import { Fragment, type ReactNode, useEffect, useId, useState } from "react";
import { Link, useNavigate } from "react-router";
import { DocDiagBadge, DocStaleBadge, DocStatusBadge } from "@/entities/doc";
import { agentTone, displayName, ROLE_TITLE_RU } from "@/entities/member";
import type { DocsImpact, DocsImpactReason } from "@/shared/api";
import {
  ArtifactThumb,
  EpicIcon,
  historyText,
  Labels,
  PRIORITY_NAME,
  PriorityIcon,
  type Status,
  STATUS_NAME,
  StatusIcon,
  statusNote,
  type Task,
  useArtifactViewer,
  useCheck,
  useComment,
  useDeleteTask,
  useDocsImpact,
  useEpicMap,
  useMoveTask,
  usePatchTask,
  useTask,
  useTasks,
} from "@/entities/task";
import { liveTeam, useAgentConfig } from "@/entities/agent-config";
import { PersonAvatar, responsibleChoices, useMembers } from "@/entities/project";
import { canWriteProject, useSession } from "@/entities/session";
import { type Team, memberActivity, PartyAvatar, useTeam } from "@/entities/team";
import { SpendText, useTaskUsage } from "@/entities/usage";
import { IDEA_TEMPLATE } from "@/features/shape-idea";
import { OwnerDecision } from "@/features/owner-decision";
import { SpawnTeamDialog } from "@/features/spawn-team";
import { plural, readPref, timeAgo, useTick, writePref } from "@/shared/lib";
import {
  Avatar as KitAvatar,
  Banner,
  Button,
  Character,
  ConfirmDialog,
  EmptyState,
  ErrorState,
  Icon,
  IconButton,
  Input,
  Markdown,
  Segments,
  Select,
  Skeleton,
  Tag,
  useToast,
} from "@/shared/ui";
import { FollowTask } from "@/features/follow-task";
import { TaskDelivery } from "./TaskDelivery.tsx";
import { TaskManager } from "./TaskManager.tsx";
import { TaskObservation } from "./TaskObservation.tsx";
import { CollapsibleSection } from "./CollapsibleSection.tsx";
// The task panel renders its own cascade: detail first, then the sheets that refine parts of it,
// then the responsive and centring layers. `docs.css` covers the doc-impact box, `team.css` the
// team sheet inside the panel and `epics.css` the epic box.
import "./task-detail-sheets.css";

const KIND_NAME: Record<string, string> = { note: "заметка", progress: "прогресс", question: "вопрос", decision: "решение", review: "ревью", handoff: "передача", owner: "владелец" };

export type TaskTab = "about" | "team";

/**
 * A task: the preview beside a list or board (`variant="panel"`), or its own page
 * (`variant="page"`) — the description with every property in a floating rail. The
 * team keeps its own route and opens from the rail.
 */
export function TaskDetail({ id, team, onClose, variant = "panel", tab = "about", teamPane }: {
  id: string;
  team?: Team;
  onClose: () => void;
  variant?: "panel" | "page";
  tab?: TaskTab;
  /** The team tab's content; the page passes it in so this widget stays below the pages. */
  teamPane?: ReactNode;
}) {
  useTick();
  const q = useTask(id);
  const navigate = useNavigate();
  const toast = useToast();
  const move = useMoveTask();
  const patch = usePatchTask();
  const comment = useComment();
  const check = useCheck();
  const del = useDeleteTask();
  const [deleting, setDeleting] = useState(false);
  const tabsId = useId();
  const [moreProperties, setMoreProperties] = useState(false);
  const [draft, setDraft] = useState("");
  const [contentTab, setContentTab] = useState<"description" | "activity" | "artifacts" | "team">("description");
  const [editDesc, setEditDesc] = useState<string | undefined>();
  const [editLabels, setEditLabels] = useState<string | undefined>();
  const [spawning, setSpawning] = useState(false);
  const agents = useAgentConfig();
  const epics = useEpicMap();
  const allTasks = useTasks().data;
  const impactEnabled = q.data?.status === "review" || q.data?.status === "done";
  const impact = useDocsImpact(id, impactEnabled);
  const session = useSession().data;
  const me = session?.mode === "users" ? session.user.login : undefined;
  const myRole = session?.projects.find((p) => p.slug === session.project)?.role;
  const canDelete = myRole === "admin" || myRole === "owner";
  const writable = canWriteProject(session);
  const members = useMembers(me ? session?.project : undefined).data;

  useEffect(() => {
    setDraft("");
    setContentTab("description");
    setMoreProperties(false);
    setEditDesc(undefined);
    setEditLabels(undefined);
    setDeleting(false);
  }, [id]);

  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const viewer = useArtifactViewer(fail);

  const Shell = variant === "page" ? "main" : "aside";
  if (q.isPending) return <Shell className={variant === "page" ? "main task-page" : "detail"} aria-busy="true" aria-label="Задача загружается"><TaskSkeleton page={variant === "page"} /></Shell>;
  if (q.isError)
    return (
      <Shell className={variant === "page" ? "main task-page" : "detail"}>
        <div className="td-state">
          <ErrorState message={q.error.message} onRetry={() => void q.refetch()} />
          <Button variant="ghost" sm onClick={onClose}>Закрыть</Button>
        </div>
      </Shell>
    );
  const t: Task = q.data;
  const isEpic = t.type === "epic";
  const canSpawn = agents.isSuccess && !isEpic && team?.state !== "active" && !["done", "cancelled"].includes(t.status);
  const epic = t.parent ? epics.get(t.parent) : undefined;
  const epicChoices = [...epics.values()].filter((e) => e.id === t.parent || (e.status !== "done" && e.status !== "cancelled"));
  const people = responsibleChoices(members ?? [], t.assignee);
  const responsibleName = members?.find((m) => m.user.login === t.assignee)?.user.name;
  const assign = (login: string) =>
    patch.mutate(
      { id: t.id, patch: { assignee: login || null } },
      { onSuccess: () => toast(login ? `Ответственный за ${t.id}: @${login}` : `У ${t.id} больше нет ответственного`), onError: fail },
    );

  const setStatus = (status: Status, note?: string) =>
    move.mutate({ id: t.id, status, note }, { onSuccess: () => toast(`${t.id} → ${STATUS_NAME[status]} · оркестратор уведомлён`), onError: fail });

  // A status change's note is also a comment (`[in_progress → review] …`): the history
  // line already says it, so the comment is left out of the activity.
  const moves = new Set(t.history.filter((h) => h.event === "status").map((h) => `${h.from}>${h.to}`));
  const activity = [
    ...t.history.map((h) => ({ kind: "event" as const, at: h.at, h })),
    ...t.comments
      .filter((c) => {
        const n = statusNote(c.text);
        return !(n && moves.has(`${n.from}>${n.to}`));
      })
      .map((c) => ({ kind: "comment" as const, at: c.at, c })),
  ].sort((a, b) => a.at.localeCompare(b.at));

  const page = variant === "page";
  const pageLink = `/task/${encodeURIComponent(t.id)}`;
  const last = activity.at(-1);
  const sameStatusTasks = (allTasks ?? []).filter((task) => task.id !== t.id && task.status === t.status && task.type === t.type);

  // On its own page the task title is the screen's title; in the side panel it stays a section heading.
  const TitleTag = page ? "h1" : "h2";
  const title = (
    <TitleTag
      className={page ? "title g-page-title" : "title"}
      contentEditable={writable}
      suppressContentEditableWarning
      spellCheck={false}
      // Reachable for a programmatic hand-off (the decision box hands focus here when it closes);
      // it stays out of the Tab order.
      tabIndex={-1}
      onBlur={(e) => {
        if (!writable) return;
        const title = e.currentTarget.textContent?.trim() ?? "";
        if (title && title !== t.title) patch.mutate({ id: t.id, patch: { title } }, { onError: fail });
      }}
      onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), e.currentTarget.blur())}
    >
      {t.title}
    </TitleTag>
  );

  // What keeps the task from moving: open dependencies, a block, a cancellation. Said in words, above the text.
  const openDeps = t.deps.filter((d) => {
    const dep = allTasks?.find((x) => x.id === d);
    return !dep || (dep.status !== "done" && dep.status !== "cancelled");
  });
  const states = (
    <>
      {t.status === "cancelled" && (
        <Banner tone="neutral" title="Задача отменена">
          Работа над ней остановлена; вернуть её можно сменой статуса.
        </Banner>
      )}
      {t.status !== "cancelled" && t.status !== "done" && openDeps.length > 0 && (
        <Banner tone="neutral" title="Ждёт другие задачи">
          Не начнётся, пока не закрыты: <TaskLinks ids={openDeps} />
        </Banner>
      )}
      {t.blocked && t.status !== "cancelled" && (
        <Banner tone="signal" title="Задача заблокирована">
          {t.blocked.reason}
        </Banner>
      )}
    </>
  );

  const crumbs = (
    <span className="crumbs">
      {page && (
        <>
          <Link to="/tasks" className="d-only">
            Задачи
          </Link>
          <span className="d-only"> / </span>
        </>
      )}
      {t.parent && (
        <>
          {epic ? <Link to={`/epic/${encodeURIComponent(t.parent)}`}>{t.parent}</Link> : t.parent}
          {" / "}
        </>
      )}
      <span className="here">{t.id}</span>
    </span>
  );


  // The headline: the id, the status as text with its shape, and the title under it.
  const headline = (
    <div className="td-headline">
      <div className="td-meta">
        {page ? crumbs : <span className="mono td-id">{t.id}</span>}
        {page && <span className="g-status">
          <StatusIcon status={t.status} size={16} />
          {STATUS_NAME[t.status]}
        </span>}
        {!page && epic && <Link className="td-header-epic" to={`/epic/${encodeURIComponent(epic.id)}`} title={`${epic.id} · ${epic.title}`}><EpicIcon size={13} />{epic.id} · {epic.title}</Link>}
        {isEpic && <Tag tone="outline">эпик</Tag>}
        {!page && (
          <span className="td-meta-more">
            <PriorityIcon priority={t.priority} size={14} />
            {PRIORITY_NAME[t.priority]}
          </span>
        )}
      </div>
      {page ? title : <div className="td-title-row">{title}{t.labels.length > 0 && <div className="td-header-labels" aria-label="Метки задачи"><Labels labels={t.labels} /></div>}</div>}
    </div>
  );

  // The properties read top-down as fields: a label with the value's shape, then the value itself,
  // which is a control only for someone allowed to write. The page keeps the whole set; the
  // preview only what a glance needs.
  const primaryProperties = (
    <>
      <Prop label="Статус" icon={<StatusIcon status={t.status} size={13} />}>
        {writable ? (
          <Select sm aria-label="Статус" value={t.status} onChange={(e) => setStatus(e.target.value as Status)}>
            {(Object.keys(STATUS_NAME) as Status[]).map((s) => (
              <option key={s} value={s}>
                {!page && CSS.supports("appearance", "base-select") ? <><span aria-hidden="true"><StatusIcon status={s} size={14} /></span><span>{STATUS_NAME[s]}</span></> : STATUS_NAME[s]}
              </option>
            ))}
          </Select>
        ) : (
          <span className="td-val">{STATUS_NAME[t.status]}</span>
        )}
      </Prop>
      <Prop label="Приоритет" icon={<PriorityIcon priority={t.priority} size={13} />}>
        {writable ? (
          <Select sm aria-label="Приоритет" value={t.priority} onChange={(e) => patch.mutate({ id: t.id, patch: { priority: Number(e.target.value) } }, { onError: fail })}>
            {PRIORITY_NAME.map((p, i) => (
              <option key={p} value={i}>
                {p}
              </option>
            ))}
          </Select>
        ) : (
          <span className="td-val">{PRIORITY_NAME[t.priority]}</span>
        )}
      </Prop>
      {(people.length > 0 || t.assignee) && (
        <Prop label="Ответственный" icon={t.assignee ? <PersonAvatar login={t.assignee} name={responsibleName} prefix="" /> : undefined}>
          {writable ? (
            <Select sm aria-label="Ответственный" value={t.assignee ?? ""} onChange={(e) => assign(e.target.value)} title="Получает вопросы агентов">
              <option value="">Не назначен</option>
              {people.map((p) => (
                <option key={p.login} value={p.login}>
                  {p.login === me ? `${p.label} — я` : p.label}
                </option>
              ))}
            </Select>
          ) : (
            <span className="td-val">{responsibleName ?? t.assignee ?? "Не назначен"}</span>
          )}
        </Prop>
      )}
    </>
  );
  const epicProperty = (
    <>
      {!isEpic && (
        <Prop label="Эпик" icon={<EpicIcon size={13} empty={!epic} />}>
          {writable ? (
            <Select
              sm
              aria-label="Эпик"
              value={epic ? epic.id : ""}
              onChange={(e) => {
                // Read the chosen parent once, before the request: the controlled select keeps showing the
                // saved epic until the refetch lands, so the callback would name the previous epic.
                const parent = e.target.value || null;
                const taskId = t.id;
                patch.mutate(
                  { id: taskId, patch: { parent } },
                  { onSuccess: () => toast(parent ? `${taskId} теперь в эпике ${parent}` : `${taskId} больше не в эпике`), onError: fail },
                );
              }}
            >
              <option value="">Без эпика</option>
              {epicChoices.map((e) => (
                <option key={e.id} value={e.id}>
                  {e.id} · {e.title}
                </option>
              ))}
            </Select>
          ) : (
            <span className="td-val">{epic ? `${epic.id} · ${epic.title}` : "Без эпика"}</span>
          )}
        </Prop>
      )}
    </>
  );

  const props = (
    <div className={page ? "td-fields rail" : "td-fields"}>
      {page && primaryProperties}
      {page && epicProperty}
      {(t.deps.length > 0 || (!isEpic && t.children.length > 0)) && (
        <Prop label="Связи">
          <span className="td-val muted">
            {t.deps.length > 0 && (
              <>
                зависит от <TaskLinks ids={t.deps} />
              </>
            )}
            {t.deps.length > 0 && !isEpic && t.children.length > 0 && " · "}
            {!isEpic && t.children.length > 0 && (
              <>
                подзадачи <TaskLinks ids={t.children} />
              </>
            )}
          </span>
        </Prop>
      )}
      {page && (
        <Prop label="Метки">
          {editLabels === undefined ? (
            <>
              {t.labels.length ? <Labels labels={t.labels} /> : <span className="td-val muted">нет</span>}
              {writable && <IconButton label="Изменить метки" onClick={() => setEditLabels(t.labels.join(", "))}><Icon.pencil size={14} /></IconButton>}
            </>
          ) : (
            <form className="td-label-editor" onSubmit={(event) => {
              event.preventDefault();
              const labels = [...new Set(editLabels.split(",").map((label) => label.trim()).filter(Boolean))];
              patch.mutate({ id: t.id, patch: { labels } }, { onSuccess: () => setEditLabels(undefined), onError: fail });
            }}>
              <Input sm autoFocus aria-label="Метки через запятую" placeholder="Метки через запятую" value={editLabels} onChange={(event) => setEditLabels(event.target.value)} />
              <div className="td-label-actions">
                <Button type="button" variant="ghost" sm onClick={() => setEditLabels(undefined)}>Отмена</Button>
                <Button type="submit" variant="secondary" sm disabled={patch.isPending}>Сохранить метки</Button>
              </div>
            </form>
          )}
        </Prop>
      )}
      {page && (
        <>
          <Prop label="Интеграция">
            {writable ? (
              <Input
                sm
                key={`${t.id}-merge-${t.mergeStrategy}`}
                aria-label="Интеграция"
                defaultValue={t.mergeStrategy}
                placeholder="не задана"
                onBlur={(e) => e.target.value !== t.mergeStrategy && patch.mutate({ id: t.id, patch: { mergeStrategy: e.target.value } }, { onError: fail })}
              />
            ) : (
              <span className={t.mergeStrategy ? "td-val" : "td-val muted"}>{t.mergeStrategy || "не задана"}</span>
            )}
          </Prop>
          {t.worktree && (
            <Prop label="Ветка">
              <span className="td-val mono td-branch" title={t.worktree.path}>
                {t.worktree.branch ?? t.worktree.path}
              </span>
            </Prop>
          )}
          <TaskSpend id={t.id} epic={isEpic} />
        </>
      )}
    </div>
  );

  const description = (
    <CollapsibleSection
      title={isEpic ? "Цель" : "Описание"}
      action={editDesc === undefined && writable ? (
        <IconButton label={isEpic ? "Изменить цель" : "Изменить описание"} className="sec-edit" onClick={() => setEditDesc(t.description)}>
          <Icon.pencil size={14} />
        </IconButton>
      ) : null}
    >
      {editDesc === undefined ? (
        <Markdown text={t.description} empty="Описания нет" />
      ) : (
        <div className="composer">
          <textarea className="g-textarea" aria-label="Описание" rows={8} value={editDesc} onChange={(e) => setEditDesc(e.target.value)} />
          <div className="bar">
            markdown
            <span className="grow" />
            <Button variant="ghost" sm onClick={() => setEditDesc(undefined)}>
              Отмена
            </Button>
            <Button
              variant="secondary"
              sm
              onClick={() => patch.mutate({ id: t.id, patch: { description: editDesc } }, { onSuccess: () => setEditDesc(undefined), onError: fail })}
            >
              Сохранить
            </Button>
          </div>
        </div>
      )}
    </CollapsibleSection>
  );

  const criteriaDone = t.acceptance.filter((a) => a.done).length;
  const criteria = (
    <CollapsibleSection title={isEpic ? "Критерии успеха" : "Критерии приёмки"} badge={
        <span className="n">
          {criteriaDone} из {t.acceptance.length}
        </span>
      }>
      {t.acceptance.length ? (
        <div className="criteria">
          <Segments total={t.acceptance.length} done={criteriaDone} label="Критерии" />
          {t.acceptance.map((a) => (
            <label key={a.id} className={`g-check${a.done ? " done" : ""}`}>
              <input type="checkbox" checked={a.done} disabled={!writable} onChange={(e) => check.mutate({ id: t.id, n: a.id, done: e.target.checked }, { onError: fail })} />
              <span className="t">{a.text}</span>
              {a.checkedBy && <span className="by">{a.checkedBy}</span>}
            </label>
          ))}
        </div>
      ) : (
        <span className="muted">Оркестратор сформулирует критерии при уточнении</span>
      )}
    </CollapsibleSection>
  );

  const epicLink = isEpic && (
    <Link className="epic-box link" to={`/epic/${encodeURIComponent(t.id)}`}>
      <EpicIcon />
      Это эпик: его задачи, прогресс и общие артефакты — на странице эпика
      <Icon.chevron size={12} />
    </Link>
  );

  const dialogs = (
    <>
      {viewer.modal}
      {spawning && <SpawnTeamDialog task={t} onClose={() => setSpawning(false)} />}
      {deleting && (
        <ConfirmDialog
          title={`Удалить ${t.id} навсегда?`}
          confirmLabel="Удалить"
          danger
          busy={del.isPending}
          onClose={() => setDeleting(false)}
          onConfirm={() =>
            del.mutate(
              { id: t.id, cascade: t.children.length > 0 },
              {
                onSuccess: (r) => {
                  toast(r.deleted.length > 1 ? `Удалено задач: ${r.deleted.length}` : `${t.id} удалена`);
                  setDeleting(false);
                  onClose();
                },
                onError: fail,
              },
            )
          }
        >
          «{t.title}» исчезнет вместе с комментариями, артефактами и историей — восстановить не получится.
          {t.children.length > 0 && ` Подзадачи (${t.children.length}) будут удалены вместе с ней.`}
          {t.team && " Команда, которая над ней работает, будет остановлена и удалена (ветка git останется)."}
        </ConfirmDialog>
      )}
    </>
  );


  const contentTabs = (
            <div className="tp-content-tabs" role="tablist" aria-label="Содержимое задачи">
              {([ ["description", "Описание"], ["activity", "Активность"], ["artifacts", "Вложения"], ...(!page && !isEpic ? [["team", "Команда"] as const] : []) ] as const).map(([value, label], index, tabs) => (
                <button key={value} type="button" role="tab" id={`${tabsId}-tab-${value}`} aria-controls={`${tabsId}-content-${value}`} aria-selected={contentTab === value} tabIndex={contentTab === value ? 0 : -1} onClick={() => setContentTab(value)} onKeyDown={(event) => {
                  if (event.key !== "ArrowRight" && event.key !== "ArrowLeft" && event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
                  event.preventDefault();
                  const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : -1) + tabs.length) % tabs.length;
                  setContentTab(tabs[next][0]);
                  document.getElementById(`${tabsId}-tab-${tabs[next][0]}`)?.focus();
                }}>{label}{value === "artifacts" && t.artifacts.length > 0 && <span>{t.artifacts.length}</span>}</button>
              ))}
            </div>
  );

  const contentPanels = (
    <>
            <div className="tp-tab-content" role="tabpanel" id={`${tabsId}-content-description`} aria-labelledby={`${tabsId}-tab-description`} hidden={contentTab !== "description"}>
            {description}
            {criteria}
            {!isEpic && <TaskDelivery task={t.id} />}
            {!isEpic && <TaskManager key={`manager-${t.id}`} task={t} />}
            {!isEpic && <TaskObservation key={`observation-${t.id}`} task={t} />}
            {!isEpic && <TaskIntelligence task={t.id} />}
            {t.plan.trim() && (
              <CollapsibleSection title={isEpic ? "Дорожная карта" : "План"}>
                <Markdown text={t.plan} />
              </CollapsibleSection>
            )}
            {t.notes.trim() && (
              <CollapsibleSection className="notes" title="Заметки">
                <Markdown text={readableStamps(t.notes)} />
              </CollapsibleSection>
            )}
            </div>
            <div className="tp-tab-content" role="tabpanel" id={`${tabsId}-content-artifacts`} aria-labelledby={`${tabsId}-tab-artifacts`} hidden={contentTab !== "artifacts"}>
            {t.artifacts.length > 0 ? (
              <CollapsibleSection title="Артефакты" badge={<span className="n">{t.artifacts.length}</span>}>
                <div className="artifacts">
                  {t.artifacts.map((a) => (
                    <button type="button" key={a.id} className="artifact g-card flush" onClick={() => viewer.show(t.id, a.id)}>
                      <span className="thumb">
                        <ArtifactThumb task={t.id} artifact={a} />
                      </span>
                      <span className="cap">
                        <span className="nm">{a.name}</span>
                        <span className="who">
                          {a.kind} · {a.author}
                        </span>
                      </span>
                    </button>
                  ))}
                </div>
              </CollapsibleSection>
            ) : <p className="muted">Вложений пока нет</p>}
            </div>
            <div className="tp-tab-content tp-activity-panel" role="tabpanel" id={`${tabsId}-content-activity`} aria-labelledby={`${tabsId}-tab-activity`} hidden={contentTab !== "activity"}>
            <section className="tp-activity">
              <div className="timeline">
                {activity.map((a) =>
                  a.kind === "event" ? (
                    <div key={`h${a.at}${a.h.event}`} className="ev">
                      <i />
                      <span className="txt">
                        <span className="who">{a.h.actor}</span> {historyText(a.h, isEpic)}
                      </span>
                      <span className="when">{timeAgo(a.at)}</span>
                    </div>
                  ) : (
                    <div key={`c${a.c.id}`} className={`comment g-sheet${a.c.role === "human" ? " mine" : ""}`}>
                      <div className="head">
                        <KitAvatar kind={a.c.role === "human" ? "human" : "agent"} name={a.c.author} sm tone={agentTone(a.c.role)} />
                        <span className="who">{a.c.author}</span>
                        <span className="k">{KIND_NAME[a.c.kind] ?? a.c.kind}</span>
                        <span className="when">{timeAgo(a.c.at)}</span>
                      </div>
                      <Markdown text={a.c.text} />
                    </div>
                  ),
                )}
              </div>
              {writable ? (
                <div className="composer">
                  <textarea
                    className="g-textarea"
                    aria-label="Комментарий"
                    placeholder="Написать комментарий"
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && draft.trim()) comment.mutate({ id: t.id, text: draft.trim() }, { onSuccess: () => setDraft(""), onError: fail });
                    }}
                  />
                  <div className="bar">
                    <span className="grow" />
                    <Button
                      variant="secondary"
                      sm
                      disabled={!draft.trim() || comment.isPending}
                      onClick={() => comment.mutate({ id: t.id, text: draft.trim() }, { onSuccess: () => setDraft(""), onError: fail })}
                    >
                      Отправить
                    </Button>
                  </div>
                </div>
              ) : (
                <p className="muted">У вас доступ только для чтения: комментировать нельзя.</p>
              )}
            </section>
            </div>
            {!page && !isEpic && <div className="tp-tab-content" role="tabpanel" id={`${tabsId}-content-team`} aria-labelledby={`${tabsId}-tab-team`} hidden={contentTab !== "team"}>
              <TeamCard task={t.id} status={t.status} team={team} canSpawn={canSpawn && writable} onSpawn={() => setSpawning(true)} compact />
            </div>}
    </>
  );

  if (!page)
    return (
      <aside className="detail" aria-label={`Задача ${t.id}`}>
        <header className="td-bar">
          <IconButton label="Назад" className="m-only" onClick={onClose}>
            <Icon.back />
          </IconButton>
          {headline}
          <div className="td-header-actions">
            <FollowTask task={t.id} />
            <Link className="g-icon-btn" to={pageLink} aria-label="Открыть страницу" title="Открыть страницу">
              <Icon.external size={16} />
            </Link>
            <IconButton label="Закрыть" className="d-only" onClick={onClose}>
              <Icon.close />
            </IconButton>
          </div>
        </header>

        <div className="detail-body">
          <div className="detail-property-bar" aria-label="Основные свойства задачи">
            {primaryProperties}
            {(t.deps.length > 0 || (!isEpic && t.children.length > 0)) && <IconButton label="Ещё свойства" aria-expanded={moreProperties} aria-controls={`${tabsId}-more-properties`} onClick={() => setMoreProperties(!moreProperties)}><Icon.dots size={16} /></IconButton>}
          </div>
          <div className="detail-extra-properties tp-card" id={`${tabsId}-more-properties`} hidden={!moreProperties}>
            {props}
          </div>
          <div className="detail-main">
            {states}
            {epicLink}
            {!t.needsOwner?.question.startsWith("Clarification #") && <OwnerDecision task={t} />}
            {contentTabs}
            {contentPanels}
          </div>
          <footer className="detail-more">
            {last && <span className="muted">Последнее: {last.kind === "event" ? `${last.h.actor} ${historyText(last.h, isEpic)}` : `${last.c.author}, ${KIND_NAME[last.c.kind] ?? last.c.kind}`} · {timeAgo(last.at)}</span>}
            <span className="grow" />
            <Link to={pageLink}>Открыть страницу задачи →</Link>
          </footer>
        </div>
        {dialogs}
      </aside>
    );

  return (
    <main className="main task-page" aria-label={`Задача ${t.id}`}>
      <header className="td-bar tp-page-header">
        <IconButton label={tab === "team" ? "К описанию задачи" : "Назад"} className="tp-back" onClick={() => {
          if (tab === "team") navigate(pageLink);
          else if (typeof window.history.state?.idx === "number" && window.history.state.idx > 0) navigate(-1);
          else navigate("/tasks");
        }}>
          <Icon.back />
        </IconButton>
        {headline}
        <div className="td-header-actions">
          <FollowTask task={t.id} />
          {team?.template === IDEA_TEMPLATE && team.state === "active" && team.members[0] && (
            <Link to={teamLink(team)} className="td-link">Разговор о плане</Link>
          )}
          {canDelete && (
            <IconButton label="Удалить задачу" onClick={() => setDeleting(true)}>
              <Icon.trash />
            </IconButton>
          )}
        </div>
      </header>

      {tab === "team" && !isEpic ? (
        (teamPane ?? (
          <div className="tp-noteam">
            <EmptyState
              title="Команда не собрана"
              character="sleep"
              characterLabel="Команда ещё не собрана"
              action={canSpawn && writable ? <Button variant="primary" onClick={() => setSpawning(true)}>Собрать команду</Button> : undefined}
            >
              Соберите команду агентов: здесь появятся её почта, схема и участники.
            </EmptyState>
          </div>
        ))
      ) : (
        <div className="tp-body">
          <aside className="tp-navigation" aria-label="Навигация по задаче">
            {contentTabs}
            {epic && (
              <section className="tp-related" aria-label="Родительский эпик">
                <h2>Эпик</h2>
                <Link className="tp-related-task" to={`/epic/${encodeURIComponent(epic.id)}`}>
                  <span>{epic.id}</span><b>{epic.title}</b>
                </Link>
              </section>
            )}
            {sameStatusTasks.length > 0 && (
              <section className="tp-related" aria-label="Задачи с тем же статусом">
                <h2>Тот же статус <span>{sameStatusTasks.length}</span></h2>
                <div className="tp-related-list">
                  {sameStatusTasks.slice(0, 3).map((task) => (
                    <Link key={task.id} className="tp-related-task" to={`/task/${encodeURIComponent(task.id)}`}>
                      <span>{task.id}</span><b>{task.title}</b>
                    </Link>
                  ))}
                  {sameStatusTasks.length > 3 && (
                    <details>
                      <summary>Ещё {sameStatusTasks.length - 3}</summary>
                      {sameStatusTasks.slice(3).map((task) => (
                        <Link key={task.id} className="tp-related-task" to={`/task/${encodeURIComponent(task.id)}`}>
                          <span>{task.id}</span><b>{task.title}</b>
                        </Link>
                      ))}
                    </details>
                  )}
                </div>
              </section>
            )}
          </aside>
          <div className="tp-main">
            {states}
            {epicLink}
            {impactEnabled && <DocsImpactBlock result={impact.data} />}
            {!t.needsOwner?.question.startsWith("Clarification #") && <OwnerDecision task={t} />}
            {contentPanels}
          </div>
          <aside className="tp-rail" aria-label="Свойства задачи">
            <RailCard id="properties" title="Свойства" className="tp-properties">
              {props}
            </RailCard>
            <TeamCard task={t.id} status={t.status} team={team} canSpawn={canSpawn && writable} onSpawn={() => setSpawning(true)} />
          </aside>
        </div>
      )}
      {dialogs}
    </main>
  );
}

/** Where a team opens: an idea being shaped goes straight to its planner's conversation. */
function teamLink(team: Team): string {
  if (team.template === IDEA_TEMPLATE && team.state === "active" && team.members[0]) return `/team/${encodeURIComponent(team.id)}/${encodeURIComponent(team.members[0].name)}`;
  return `/task/${encodeURIComponent(team.task)}/team`;
}

/** A phone-sized window; matches the 640px breakpoint of the design system and follows resizes. */
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.matchMedia("(max-width: 640px)").matches);
  useEffect(() => {
    const m = window.matchMedia("(max-width: 640px)");
    const on = () => setNarrow(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, []);
  return narrow;
}

/** A rail card that folds to its title; the choice is remembered per card. */
function RailCard({ id, title, className, children, collapsible = true }: { id: string; title: string; className?: string; children: ReactNode; collapsible?: boolean }) {
  const key = `genie.railCard.${id}`;
  const contentId = useId();
  // On a phone the properties fold under the headline like an accordion until opened.
  const narrow = useNarrow();
  const [open, setOpen] = useState(() => readPref(key, id === "properties" && narrow ? "closed" : "open") === "open");
  if (!collapsible) return <section className={`tp-card${className ? ` ${className}` : ""}`}>{title && <h3 className="tp-card-title">{title}</h3>}<div className="tp-card-body">{children}</div></section>;
  return (
    <section className={`tp-card${className ? ` ${className}` : ""}${open ? "" : " collapsed"}`}>
      <button
        type="button"
        className="tp-card-toggle"
        aria-expanded={open}
        aria-controls={contentId}
        title={open ? `Свернуть «${title}»` : `Развернуть «${title}»`}
        onClick={() => {
          writePref(key, open ? "closed" : "open");
          setOpen(!open);
        }}
      >
        <span className="tp-card-title">{title}</span>
        <Icon.chevron size={12} />
      </button>
      <div className="tp-card-body" id={contentId} hidden={!open}>
        {children}
      </div>
    </section>
  );
}

function TeamCard({ task, status, team, canSpawn, onSpawn, compact = false }: { task: string; status: Status; team?: Team; canSpawn: boolean; onSpawn: () => void; compact?: boolean }) {
  const detail = useTeam(team?.id).data;
  const current = detail ?? team;
  const spec = detail?.spec;
  const active = current?.state === "active";
  const live = current && spec ? liveTeam(spec, current.members, status, active, displayName).live : {};
  return (
    <RailCard id="team" title={compact ? "" : "Команда"} className={`tp-team-card${compact ? " tp-team-compact" : ""}`} collapsible={!compact}>
      {current?.members.length ? (
        <>
          <div className="tp-team-roster" aria-label="Участники команды">
            {current.members.map((member) => {
              const key = spec?.members.find((entry) => entry.name === member.name)?.key ?? member.name;
              const activity = memberActivity(current, member, live[key], active ? detail?.sessions?.[member.name] : undefined);
              const queued = detail?.pending[member.name] ?? 0;
              return (
                <Link className={`tp-member ${activity.state}`} key={member.name} to={`/team/${encodeURIComponent(current.id)}/${encodeURIComponent(member.name)}`}>
                  <PartyAvatar role={member.role} name={member.name} activity={active ? member.activity : undefined} state={member.state} />
                  <span className="tp-member-info">
                    <span className="tp-member-name"><b>{displayName(member.name)}</b><span>{ROLE_TITLE_RU[member.role] ?? member.role}</span></span>
                    <span className="tp-member-state">{activity.text}{queued > 0 && <span title="Писем ждёт доставки"> · ✉ {queued}</span>}</span>
                  </span>
                  <span className="tp-member-when">{timeAgo(member.activityAt ?? member.statusAt)}</span>
                </Link>
              );
            })}
          </div>
          <Link className="g-btn secondary sm" to={`/task/${encodeURIComponent(task)}/team`}>
            Перейти к команде <Icon.chevron size={12} />
          </Link>
        </>
      ) : compact ? (
        <div className="tp-team-compact-empty">
          <span className="muted">Команда не собрана</span>
          {canSpawn && <Button variant="secondary" sm onClick={onSpawn}><Icon.userPlus size={12} />Собрать команду</Button>}
        </div>
      ) : (
        <div className="tp-team-empty">
          <Character state="sleep" size={56} />
          <b>Команда не собрана</b>
          <Link className="g-btn secondary sm" to={`/task/${encodeURIComponent(task)}/team`}>
            Перейти к команде <Icon.chevron size={12} />
          </Link>
          {canSpawn && <Button variant="primary" sm onClick={onSpawn}><Icon.userPlus size={12} />Собрать команду</Button>}
        </div>
      )}
    </RailCard>
  );
}

/** Task ids as links to their pages. */
function TaskLinks({ ids }: { ids: string[] }) {
  return (
    <>
      {ids.map((id, i) => (
        <Fragment key={id}>
          {i > 0 && ", "}
          <Link className="mono" to={`/task/${encodeURIComponent(id)}`}>
            {id}
          </Link>
        </Fragment>
      ))}
    </>
  );
}

/** Notes carry raw ISO stamps (`2026-10-03T14:32:58.935Z`): shown as a short local date and time. */
function readableStamps(text: string): string {
  return text.replace(/\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?Z\b/g, (iso) => {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  });
}

/** Russian phrasing of the structured impact reasons; matching stays server-side (G-12 lesson). */
function impactReasonText(reasons: DocsImpactReason[]): string {
  return reasons
    .map((reason) => (reason.kind === "changed-path" ? `меняет ${reason.path} — под paths: ${reason.pattern}` : `ссылается на задачу ${reason.id} в related`))
    .join("; ");
}

/**
 * Russian phrasing of the degradation notes (G-39 F2). The core keeps its note
 * vocabulary stable and English for the CLI and logs, so the only Russian surface
 * maps that fixed vocabulary here. An unmapped note falls back to a generic
 * Russian line, so no English text ever becomes visible in the UI.
 */
function impactNoteText(note: string): string {
  const mapping: [RegExp, (m: RegExpExecArray) => string][] = [
    [/^no team worktree for this task$/, () => "у задачи нет рабочей копии команды"],
    [/^worktree (.+) does not exist$/, (m) => `рабочая копия ${m[1]} не найдена`],
    [/^worktree (.+) is not a git working tree$/, (m) => `${m[1]} — не git-рабочая копия`],
    [/^worktree (.+) could not be inspected: .*$/, (m) => `рабочую копию ${m[1]} не удалось проверить`],
    [/^no base commit recorded for the team$/, () => "для команды не записан базовый коммит"],
    [/^base (.+) is not reachable from the worktree$/, (m) => `базовый коммит ${m[1]} недоступен из рабочей копии`],
    [/^no changes found between (.+) and HEAD$/, (m) => `между ${m[1]} и HEAD изменений не найдено`],
    [/^docs index unavailable: .*$/, () => "индекс документации недоступен"],
  ];
  for (const [pattern, render] of mapping) {
    const match = pattern.exec(note);
    if (match) return render(match);
  }
  return "не удалось получить данные об изменениях";
}

/** Mockup screen 9: pages the task's changes may have made stale. A hint, never a gate. */
function DocsImpactBlock({ result }: { result: DocsImpact | undefined }) {
  const navigate = useNavigate();
  if (!result) return null;
  const candidates = result.candidates;
  const note = result.notes[0];
  return (
    <section className="doc-impact" aria-label="Документация, которую могла затронуть задача">
      <div className="h">
        <Icon.file size={14} />
        Документация, которую могла затронуть задача
        {candidates.length > 0 && <span className="n">{candidates.length}</span>}
      </div>
      {candidates.length > 0 ? (
        <div className="rows">
          {candidates.map((candidate) => (
            <button
              type="button"
              key={candidate.path}
              className="row"
              onClick={() => navigate(`/docs?page=${encodeURIComponent(candidate.path)}`)}
            >
              {/*
                Screen 9: title + badges on the first line, the full reason on its
                own wrapping line, so the matched pattern is never cut off (G-39 F1).
                F5: `DocMarks` from `@/entities/doc` renders icon-only tree/search
                marks and always draws a status icon (including «актуальна»/«без
                статуса»), while the mockup wants text badges and only for
                draft/deprecated — so the block reuses that module's badge
                components instead of `DocMarks`.
              */}
              <span className="line">
                <Icon.file size={13} />
                <span className="nm">{candidate.title}</span>
                {(candidate.status === "draft" || candidate.status === "deprecated") && <DocStatusBadge status={candidate.status} />}
                {candidate.stale && <DocStaleBadge count={candidate.staleReasons.length} />}
                {candidate.diagnostics.length > 0 && <DocDiagBadge count={candidate.diagnostics.length} />}
              </span>
              <span className="why">{impactReasonText(candidate.reasons)}</span>
            </button>
          ))}
        </div>
      ) : (
        <div className="muted">Затронутой документации не найдено</div>
      )}
      {note && (
        <div className="why note">
          {result.changedPathsAvailable ? `замечание: ${impactNoteText(note)}` : `нет данных об изменениях: ${impactNoteText(note)}`}
        </div>
      )}
    </section>
  );
}


/** A property of the task: the label with the value's shape, then the value under it. */
function Prop({ label, icon, children }: { label: string; icon?: ReactNode; children: ReactNode }) {
  return (
    <div className="g-field td-prop">
      <span className="g-label">
        {icon}
        {label}
      </span>
      <span className="td-prop-value">{children}</span>
    </div>
  );
}

/** What the agents spent on the task (with its subtasks; an epic, with its tasks); nothing until they spend. */
function TaskSpend({ id, epic }: { id: string; epic: boolean }) {
  const usage = useTaskUsage(id).data;
  if (!usage?.spend.calls) return null;
  const tasks = usage.tasks.length;
  return (
    <Prop label="Расходы">
      <span className="td-val">
        <SpendText spend={usage.spend} extra={tasks > 1 ? (epic ? `${tasks} ${plural(tasks, "задача", "задачи", "задач")}` : "вместе с подзадачами") : undefined} />
      </span>
    </Prop>
  );
}

/** The task's layout while it loads: the headline, the description lines, and the rail. */
function TaskSkeleton({ page }: { page: boolean }) {
  return (
    <div className={page ? "td-skeleton page" : "td-skeleton"}>
      <Skeleton width={120} height={16} />
      <Skeleton width="70%" height={page ? 36 : 24} />
      <div className="td-skeleton-body">
        <Skeleton height={14} />
        <Skeleton width="92%" height={14} />
        <Skeleton width="80%" height={14} />
        <Skeleton height={96} />
      </div>
    </div>
  );
}
