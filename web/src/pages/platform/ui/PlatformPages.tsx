// Pages for the platform features of the server: answering questions (public
// link), notifications, automations with runs and playbooks, and the review of
// knowledge proposals.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import {
  type Automation,
  type Notification,
  RUN_STATUS,
  useAutomations,
  useNotifications,
  usePlaybooks,
  useProposal,
  useProposals,
  useRun,
  useRuns,
} from "@/entities/platform";
import { request } from "@/shared/api";
import { timeAgo, useTick } from "@/shared/lib";
import { Icon, Markdown, Modal, useRefreshingAct, useToast } from "@/shared/ui";
import { describeStep, describeTrigger } from "../model/describe.ts";

// ------------------------------------------------------------------ answer page (public)

interface Questionnaire {
  id: number;
  task?: string;
  askedBy: string;
  status: string;
  questions: { n: number; text: string; why: string; options: string[]; answer?: string }[];
}

export function AnswerPage() {
  const [sp] = useSearchParams();
  const token = sp.get("token") ?? "";
  const [data, setData] = useState<{ questionnaire: Questionnaire; taskTitle?: string } | null>();
  const [error, setError] = useState<string>();
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [done, setDone] = useState(false);
  if (data === undefined && !error) {
    request<{ questionnaire: Questionnaire; taskTitle?: string }>("GET", `/api/answer/${encodeURIComponent(token)}`).then(setData, (e: Error) => setError(e.message));
  }
  const submit = async () => {
    try {
      const res = await request<{ questionnaire: Questionnaire; complete: boolean }>("POST", `/api/answer/${encodeURIComponent(token)}`, { answers });
      setData((d) => (d ? { ...d, questionnaire: res.questionnaire } : d));
      setDone(res.complete);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const qn = data?.questionnaire;
  const closed = done || (qn && qn.status !== "open");
  return (
    <div className="auth-page">
      <div className="answer">
        <span className="logo-mark lg">
          <Icon.mark size={22} />
        </span>
        {error && <div className="auth-error">{error}</div>}
        {!qn && !error && <div className="muted">Загрузка…</div>}
        {qn && (
          <>
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <span className="ctx">
                {qn.task ? <span className="mono">{qn.task}</span> : "genie"} · спрашивает {qn.askedBy}
              </span>
              <h1>{closed ? "Ответы получены" : "Нужны ваши ответы"}</h1>
              {data?.taskTitle && <p className="lead">{data.taskTitle}</p>}
            </div>
            {closed ? (
              <div className="done">
                {qn.status === "expired" ? "Срок ответа истёк — вопрос передан владельцу проекта." : "Спасибо! Ответы записаны в задачу, команда продолжит работу."}
              </div>
            ) : (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void submit();
                }}
              >
                {qn.questions.map((q) => (
                  <fieldset key={q.n}>
                    <legend>
                      <span className="n">{q.n}.</span> {q.text}
                    </legend>
                    {q.why && <span className="why">{q.why}</span>}
                    {q.answer ? (
                      <span className="answered">
                        <Icon.check size={13} />
                        {q.answer}
                      </span>
                    ) : (
                      <>
                        {q.options.map((o) => (
                          <label key={o} className={`opt${answers[q.n] === o ? " on" : ""}`}>
                            <input type="radio" name={`q${q.n}`} checked={answers[q.n] === o} onChange={() => setAnswers({ ...answers, [q.n]: o })} />
                            {o}
                          </label>
                        ))}
                        <textarea
                          rows={q.options.length ? 2 : 3}
                          aria-label={`Ответ на вопрос ${q.n}`}
                          value={q.options.includes(answers[q.n] ?? "") ? "" : (answers[q.n] ?? "")}
                          placeholder={q.options.length ? "Или свой ответ" : "Ваш ответ"}
                          onChange={(e) => setAnswers({ ...answers, [q.n]: e.target.value })}
                        />
                      </>
                    )}
                  </fieldset>
                ))}
                <button className="btn primary submit">Отправить ответы</button>
                <span className="note">Вход не нужен — ссылка работает, пока вопрос открыт</span>
              </form>
            )}
          </>
        )}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ notifications

export function NotificationsPage() {
  useTick();
  const q = useNotifications();
  const act = useRefreshingAct();
  const navigate = useNavigate();
  const items = q.data?.items ?? [];
  const days: { day: string; items: Notification[] }[] = [];
  for (const n of items) {
    const day = dayName(n.created);
    if (days.at(-1)?.day !== day) days.push({ day, items: [] });
    days.at(-1)!.items.push(n);
  }
  return (
    <main className="main">
      <header className="topbar">
        <h1>Уведомления</h1>
        {(q.data?.unread ?? 0) > 0 && <span className="sub">{q.data?.unread} непрочитанных</span>}
        <span className="grow" />
        {(q.data?.unread ?? 0) > 0 && (
          <button type="button" className="btn" onClick={() => void act(() => request("POST", "/api/notifications/read", {}))}>
            Прочитать все
          </button>
        )}
      </header>
      <div className="scroll">
        {!items.length && <div className="empty">{q.isPending ? "Загрузка…" : "Уведомлений нет"}</div>}
        {items.length > 0 && (
          <div className="notifs">
            {days.map((d) => (
              <section key={d.day} className="notif-day">
                <h2>{d.day}</h2>
                {d.items.map((n) => (
                  <button
                    type="button"
                    key={n.id}
                    className={`notif${n.readAt ? "" : " unread"}`}
                    onClick={() =>
                      void act(async () => {
                        if (!n.readAt) await request("POST", "/api/notifications/read", { id: n.id });
                        if (n.link) navigate(n.link);
                      })
                    }
                  >
                    <span className="dot" aria-label={n.readAt ? undefined : "не прочитано"} />
                    <span className="txt">
                      <b>{n.title}</b>
                      {n.body && <span className="notif-body">{n.body}</span>}
                    </span>
                    <span className="when">{timeAgo(n.created)}</span>
                  </button>
                ))}
              </section>
            ))}
          </div>
        )}
      </div>
    </main>
  );
}

function dayName(iso: string): string {
  const d = new Date(iso);
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(new Date()) - start(d)) / 86_400_000);
  if (diff <= 0) return "Сегодня";
  if (diff === 1) return "Вчера";
  return d.toLocaleDateString("ru-RU", { day: "numeric", month: "long" });
}

// ------------------------------------------------------------------ automations

export function AutomationsPage() {
  useTick();
  const rules = useAutomations();
  const playbooks = usePlaybooks();
  const [sp, setSp] = useSearchParams();
  const [editing, setEditing] = useState<{ id?: number; text: string }>();
  const [showPlaybooks, setShowPlaybooks] = useState(false);
  const act = useRefreshingAct();
  const installed = new Set((rules.data ?? []).map((r) => r.name));
  const selected = Number(sp.get("rule") ?? "") || rules.data?.[0]?.id;
  const current = rules.data?.find((r) => r.id === selected);
  return (
    <main className="main">
      <header className="topbar">
        <h1>Автоматизации</h1>
        <span className="grow" />
        {!!playbooks.data?.length && (
          <button type="button" className="btn" onClick={() => setShowPlaybooks(true)}>
            Плейбуки
          </button>
        )}
        <button type="button" className="btn primary" onClick={() => setEditing({ text: JSON.stringify(EMPTY_RULE, null, 2) })}>
          <Icon.plus size={13} />
          <span className="d-only">Новое правило</span>
        </button>
      </header>
      <div className="split">
        <section className="split-list" aria-label="Правила">
          {rules.data?.map((r) => (
            <button type="button" key={r.id} className={`pick${r.id === current?.id ? " on" : ""}`} onClick={() => setSp({ rule: String(r.id) })}>
              <span className="t">
                <span className={`state-dot ${r.enabled ? "ok" : "off"}`} />
                <span>{r.name}</span>
              </span>
              <span className="s">{describeTrigger(r.spec, r.enabled, r.dryRun)}</span>
              <span className="s">
                {r.lastRun ? `последний запуск ${timeAgo(r.lastRun.started)} · ${(RUN_STATUS[r.lastRun.status] ?? r.lastRun.status).toLowerCase()}` : "ещё не запускалось"}
              </span>
            </button>
          ))}
          {rules.isSuccess && !rules.data.length && (
            <p className="muted" style={{ margin: 0, padding: "4px 14px 8px" }}>
              Правил пока нет — начните с плейбука: готового правила для частого случая.
            </p>
          )}
        </section>
        <section className="split-main" aria-label="Правило">
          {current ? (
            <RuleView key={current.id} rule={current} onEdit={() => setEditing({ id: current.id, text: JSON.stringify(current.spec, null, 2) })} />
          ) : (
            <div className="pane-empty">
              <Icon.bolt size={22} />
              {rules.isPending ? "Загрузка…" : "Выберите правило или добавьте плейбук"}
            </div>
          )}
        </section>
      </div>
      {editing && <RuleEditor initial={editing} onClose={() => setEditing(undefined)} />}
      {showPlaybooks && (
        <Modal label="Плейбуки" onClose={() => setShowPlaybooks(false)}>
          <div className="mh">
            Плейбуки — готовые правила для частых случаев
            <span className="grow" />
            <button type="button" className="icon-btn" onClick={() => setShowPlaybooks(false)} aria-label="Закрыть">
              <Icon.close />
            </button>
          </div>
          <ul className="mb playbooks">
            {playbooks.data?.map((p) => {
              const added = installed.has(String((p.spec as { name?: string }).name ?? ""));
              return (
                <li key={p.id}>
                  <span className="t">{p.title}</span>
                  <span className="s">{describeTrigger(p.spec)}</span>
                  {added ? (
                    <span className="muted">добавлен</span>
                  ) : (
                    <button type="button" className="btn" onClick={() => void act(() => request("POST", `/api/automations/playbooks/${p.id}`), "Правило добавлено")}>
                      Добавить
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </Modal>
      )}
    </main>
  );
}

const EMPTY_RULE = {
  name: "Новое правило",
  on: { event: "task.status_changed", where: { to: "done" } },
  steps: [{ id: "tell", notify: { to: ["task.author"], title: "{{ event.task.id }} готова", text: "{{ event.task.title }}" } }],
};

type Spec = { on?: Record<string, unknown>; steps?: Record<string, unknown>[] };

function RuleView({ rule, onEdit }: { rule: Automation; onEdit: () => void }) {
  const act = useRefreshingAct();
  const cfg = useAgentConfig().data;
  const steps = ((rule.spec as Spec).steps ?? []).map((st) =>
    describeStep(st, {
      role: (id) => cfg?.roles.find((r) => r.id === id)?.title,
      template: (id) => cfg?.teams.find((t) => t.id === id)?.title,
    }),
  );
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          <h2>{rule.name}</h2>
          <span className="sub">{describeTrigger(rule.spec, rule.enabled, rule.dryRun)}</span>
        </div>
        <label className="toggle">
          <input type="checkbox" checked={rule.enabled} onChange={(e) => void act(() => request("POST", `/api/automations/${rule.id}/enabled`, { enabled: e.target.checked }))} />
          Включено
        </label>
        <button type="button" className="btn" onClick={onEdit}>
          Изменить
        </button>
        {rule.trigger === "manual" && (
          <button type="button" className="btn" onClick={() => void act(() => request("POST", `/api/automations/${rule.id}/run`, {}), "Запущено")}>
            Запустить
          </button>
        )}
      </div>
      <div className="pane-body">
        {steps.length > 0 && (
          <ol className="steps" aria-label="Шаги">
            {steps.map((st, i) => (
              <li key={i} title={st.hint}>
                <span className="k">{st.kind}</span>
                <span className="d">{st.title}</span>
                {st.note && <span className="n">{st.note}</span>}
              </li>
            ))}
          </ol>
        )}
        <Runs automation={rule.id} />
      </div>
    </>
  );
}

function RuleEditor({ initial, onClose }: { initial: { id?: number; text: string }; onClose: () => void }) {
  const agents = useAgentConfig().data;
  const [text, setText] = useState(initial.text);
  const [error, setError] = useState<string>();
  const qc = useQueryClient();
  const save = async () => {
    try {
      const spec = JSON.parse(text);
      if (initial.id) await request("PUT", `/api/automations/${initial.id}`, { spec });
      else await request("POST", "/api/automations", { spec });
      await qc.invalidateQueries();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const remove = async () => {
    if (!initial.id) return;
    await request("DELETE", `/api/automations/${initial.id}`);
    await qc.invalidateQueries();
    onClose();
  };
  return (
    <div className="overlay" onClick={onClose}>
      <div className="modal wide" onClick={(e) => e.stopPropagation()}>
        <div className="mh">
          {initial.id ? "Правило (JSON)" : "Новое правило (JSON)"}
          <span className="grow" />
          <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
            <Icon.close />
          </button>
        </div>
        <div className="mb">
          <p className="muted" style={{ margin: 0 }}>
            Триггер: <code>event</code> (+ <code>where</code>), <code>schedule</code> (cron, <code>tz</code>), <code>manual</code>, <code>webhook</code>. Шаги: task.status, task.comment, task.create,
            task.update, task.get, task.ready, notify, agent, team, ask, wait, wake_orchestrator, changelog.add, release, http. Подстановки: <code>{"{{ event.task.id }}"}</code>,{" "}
            <code>{"{{ steps.<id>.output.… }}"}</code>.
          </p>
          {agents && (
            <p className="muted" style={{ margin: 0 }}>
              Шаблоны для <code>team.template</code>: {agents.teams.map((t) => t.id).join(", ")}. Роли для <code>agent.role</code>:{" "}
              {agents.roles
                .filter((r) => r.class !== "orchestrator")
                .map((r) => r.id)
                .join(", ")}
              .
            </p>
          )}
          <textarea className="mono code-edit" rows={22} value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} />
          {error && <div className="auth-error">{error}</div>}
        </div>
        <div className="mf">
          {initial.id && (
            <button type="button" className="btn danger" onClick={() => void remove()}>
              Удалить
            </button>
          )}
          <span className="grow" />
          <button type="button" className="btn" onClick={onClose}>
            Отмена
          </button>
          <button type="button" className="btn primary" onClick={() => void save()}>
            Сохранить
          </button>
        </div>
      </div>
    </div>
  );
}

function Runs({ automation }: { automation: number }) {
  useTick();
  const runs = useRuns(automation);
  const [open, setOpen] = useState<number>();
  const run = useRun(open);
  const act = useRefreshingAct();
  return (
    <section>
      <h3>Последние запуски</h3>
      {!runs.data?.length ? (
        <p className="muted" style={{ margin: 0 }}>
          {runs.isPending ? "Загрузка…" : "Запусков пока не было."}
        </p>
      ) : (
        <div className="runs">
          {runs.data.map((r) => (
            <div key={r.id} className="run">
              <button type="button" className="run-row" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? undefined : r.id)}>
                <span className={`st ${r.status}`}>
                  {r.status === "running" ? <span className="spin" /> : <span className={`state-dot ${r.status}`} />}
                  {RUN_STATUS[r.status] ?? r.status}
                </span>
                <span className="id">#{r.id}</span>
                <span className="nt">{r.error ?? triggerNote(r.triggerKey)}</span>
                <span className="when">{timeAgo(r.started)}</span>
              </button>
              {open === r.id && run.data && (
                <ol className="run-steps">
                  {run.data.steps.map((s) => (
                    <li key={s.id}>
                      <details>
                        <summary>
                          <span className="sid">{s.stepId}</span>
                          <span>
                            {s.kind} · {RUN_STATUS[s.status] ?? s.status}
                            {s.attempt > 1 ? ` · попытка ${s.attempt}` : ""}
                            {s.error && <span className="err"> · {s.error}</span>}
                          </span>
                        </summary>
                        <pre className="view">{JSON.stringify({ input: s.input, output: s.output, wait: s.wait }, null, 2)}</pre>
                      </details>
                    </li>
                  ))}
                  {["queued", "running", "waiting"].includes(run.data.status) && (
                    <li>
                      <button type="button" className="btn" onClick={() => void act(() => request("POST", `/api/runs/${r.id}/cancel`), "Отменено")}>
                        Отменить запуск
                      </button>
                    </li>
                  )}
                </ol>
              )}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function triggerNote(key: string): string {
  const kind = key.split(":")[0];
  if (kind === "event") return `по событию #${key.split(":").at(-1)}`;
  return ({ schedule: "по расписанию", manual: "запущено вручную", webhook: "по webhook" } as Record<string, string>)[kind] ?? key;
}

// ------------------------------------------------------------------ knowledge proposals

/** Page body with the frontmatter shown apart, so the comparison reads like the page. */
function PageBody({ text }: { text: string }) {
  const m = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  return (
    <>
      {m && <pre className="frontmatter">{m[1]}</pre>}
      <Markdown text={m ? text.slice(m[0].length) : text} />
    </>
  );
}

export function ProposalsPage() {
  useTick();
  const [sp, setSp] = useSearchParams();
  const list = useProposals();
  const selected = Number(sp.get("proposal") ?? "") || list.data?.[0]?.id;
  const one = useProposal(selected);
  const act = useRefreshingAct();
  const decide = (verb: "approve" | "reject", force = false) =>
    act(async () => {
      await request("POST", `/api/docs/proposals/${selected}/${verb}`, { force });
      setSp({});
    }, verb === "approve" ? "Правка опубликована" : "Предложение отклонено");
  const p = one.data?.proposal;
  return (
    <main className="main">
      <header className="topbar">
        <h1>Предложения</h1>
        <span className="sub d-only">агенты предлагают правки знаний, владельцы раздела решают</span>
        <span className="grow" />
        <Link className="btn" to="/docs">
          К документации
        </Link>
      </header>
      <div className="split narrow">
        <section className="split-list" aria-label="Открытые предложения">
          {list.isSuccess && !list.data.length && <p className="muted" style={{ margin: 0, padding: "4px 14px" }}>Открытых предложений нет.</p>}
          {list.data?.map((x) => (
            <button type="button" key={x.id} className={`pick plain${selected === x.id ? " on" : ""}`} onClick={() => setSp({ proposal: String(x.id) })}>
              <span className="t">
                <span>{pageTitle(x.path)}</span>
              </span>
              <span className="s">
                {x.author}
                {x.authorKind === "agent" ? " (агент)" : ""}
                {x.task ? ` · по задаче ${x.task}` : ""} · {timeAgo(x.created)}
              </span>
            </button>
          ))}
        </section>
        <section className="split-main" aria-label="Предложение">
          {!p ? (
            <div className="pane-empty">
              <Icon.proposal size={22} />
              {list.isPending || (selected && one.isPending) ? "Загрузка…" : "Здесь появятся правки, которые ждут вашего решения"}
            </div>
          ) : (
            <>
              <div className="pane-head">
                <div className="ttl">
                  <h2>{pageTitle(p.path)}</h2>
                  <span className="sub">
                    <span className="mono">{p.path}</span>
                    {one.data!.owners.length > 0 ? ` · владельцы раздела: ${one.data!.owners.join(", ")}` : ""}
                  </span>
                </div>
                <button type="button" className="btn" onClick={() => void decide("reject")}>
                  Отклонить
                </button>
                <button type="button" className="btn" onClick={() => void decide("approve", true)} title="Если страница изменилась после предложения">
                  Опубликовать поверх
                </button>
                <button type="button" className="btn primary" onClick={() => void decide("approve")}>
                  Опубликовать
                </button>
              </div>
              <div className="proposal-state">
                <span className={`state-dot ${one.data!.current == null ? "running" : "ok"}`} />
                {one.data!.current == null ? "Новая страница" : "Правка существующей страницы — при конфликте сервер попросит опубликовать поверх"}
              </div>
              {p.note && <p className="proposal-note">{p.note}</p>}
              <div className="compare">
                <div>
                  <span className="hd">Сейчас</span>
                  <div className="bd">{one.data!.current ? <PageBody text={one.data!.current} /> : <p className="muted">Страницы ещё нет</p>}</div>
                </div>
                <div>
                  <span className="hd">Предложение</span>
                  <div className="bd">
                    <PageBody text={p.content ?? ""} />
                  </div>
                </div>
              </div>
            </>
          )}
        </section>
      </div>
    </main>
  );
}

function pageTitle(path: string): string {
  return path.replace(/\.md$/, "").split("/").join(" / ");
}
