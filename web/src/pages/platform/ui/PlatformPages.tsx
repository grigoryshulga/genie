// Pages for the platform features of the server: answering questions (public
// link), notifications, using the automations (the rules themselves are defined
// in the settings mode), and the review of knowledge proposals.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { type Automation, type Notification, describeTrigger, RUN_STATUS, useAutomations, useNotifications, useProposal, useProposals } from "@/entities/platform";
import { request } from "@/shared/api";
import { matchesSearch, timeAgo, useTick } from "@/shared/lib";
import { Icon, Markdown, Modal, useRefreshingAct, useToast } from "@/shared/ui";
import { ContextSearch } from "@/widgets/sidebar";
import { Runs } from "@/widgets/runs";

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
          <Icon.mark size={34} />
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
  const [search] = useSearchParams();
  const items = (q.data?.items ?? []).filter((n) => matchesSearch(search.get("q") ?? "", n.title, n.body));
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
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="scroll">
        {!items.length && <div className="empty">{q.isPending ? "Загрузка…" : search.get("q") ? "Ничего не найдено" : "Уведомлений нет"}</div>}
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

/** Using the automations: the rules with their last run, a manual run, and the run journal. */
export function AutomationsPage() {
  useTick();
  const rules = useAutomations();
  const [sp, setSp] = useSearchParams();
  const act = useRefreshingAct();
  const selected = Number(sp.get("rule") ?? "") || undefined;
  const filtered = rules.data?.filter((r) => matchesSearch(sp.get("q") ?? "", r.name, describeTrigger(r.spec, r.enabled, r.dryRun)));
  const current = filtered?.find((r) => r.id === selected);
  const selectRule = (id?: number) => setSp((previous) => {
    const next = new URLSearchParams(previous);
    if (id) next.set("rule", String(id)); else next.delete("rule");
    return next;
  });
  const runNow = (rule: Automation) => void act(() => request("POST", `/api/automations/${rule.id}/run`, {}), "Запущено");
  return (
    <main className="main">
      <header className="topbar">
        <h1>Автоматизации</h1>
        <span className="sub">запуск и наблюдение · правила меняют в настройках проекта</span>
        <span className="grow" />
        {current?.trigger === "manual" && (
          <button type="button" className="btn primary" onClick={() => runNow(current)}>
            <Icon.bolt size={13} />
            <span className="d-only">Запустить</span>
          </button>
        )}
      </header>
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="split">
        <section className="split-list" aria-label="Правила">
          {rules.data?.length ? (
            <button type="button" className={`pick${current ? "" : " on"}`} onClick={() => selectRule()}>
              <span className="t">
                <Icon.list size={13} />
                <span>Все правила · общий журнал</span>
              </span>
              <span className="s">запуски всех правил подряд, с отменой</span>
            </button>
          ) : null}
          {filtered?.map((r) => (
            <button type="button" key={r.id} className={`pick${r.id === current?.id ? " on" : ""}`} onClick={() => selectRule(r.id)}>
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
          {rules.isSuccess && rules.data.length > 0 && !filtered?.length && <p className="muted" style={{ padding: "4px 14px" }}>Ничего не найдено</p>}
          {rules.isSuccess && !rules.data.length && (
            <p className="muted" style={{ margin: 0, padding: "4px 14px 8px" }}>
              Правил пока нет — их добавляют в настройках проекта, на разделе «Автоматизации».
            </p>
          )}
        </section>
        <section className="split-main" aria-label="Запуски">
          <div className="pane-head">
            <div className="ttl">
              <h2>{current ? current.name : "Все правила"}</h2>
              <span className="sub">{current ? describeTrigger(current.spec, current.enabled, current.dryRun) : "журнал запусков всех правил"}</span>
            </div>
            {current?.trigger === "manual" && (
              <button type="button" className="btn" onClick={() => runNow(current)}>
                Запустить
              </button>
            )}
          </div>
          <div className="pane-body">
            <Runs automation={current?.id} />
          </div>
        </section>
      </div>
    </main>
  );
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
  const filtered = list.data?.filter((p) => matchesSearch(sp.get("q") ?? "", p.path, p.author, p.task));
  const selected = Number(sp.get("proposal") ?? "") || filtered?.[0]?.id;
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
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="split narrow">
        <section className="split-list" aria-label="Открытые предложения">
          {list.isSuccess && !list.data.length && <p className="muted" style={{ margin: 0, padding: "4px 14px" }}>Открытых предложений нет.</p>}
          {list.isSuccess && list.data.length > 0 && !filtered?.length && <p className="muted" style={{ padding: "4px 14px" }}>Ничего не найдено</p>}
          {filtered?.map((x) => (
            <button type="button" key={x.id} className={`pick plain${selected === x.id ? " on" : ""}`} onClick={() => setSp((previous) => { const next = new URLSearchParams(previous); next.set("proposal", String(x.id)); return next; })}>
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
