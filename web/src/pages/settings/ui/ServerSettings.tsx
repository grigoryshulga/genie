import { FactoryAnalytics } from "./FactoryAnalytics.tsx";
// «Управление» — global server administration, a section each: health (the doctor
// check, the week in numbers), projects, accounts, the provider keys shared with
// everyone, MCP connections and the knowledge vault sync. Server
// administrators only.
import { ProvidersAdminSection } from "./ProvidersAdmin.tsx";
import { useQueryClient } from "@tanstack/react-query";

import { useState } from "react";
import { Link, Navigate, useParams, useSearchParams } from "react-router";
import {
  AUTONOMY,
  DOCTOR_AREA,
  type DoctorCheck,
  doctorSummary,
  hoursText,
  PersonAvatar,
  personName,
  type ProjectStats,
  statsTotals,
  sumDays,
  useCreateProject,
  useCreateUser,
  useDeleteUser,
  useSetMember,
  type Person,
  useDoctor,
  usePatchUser,
  useProjects,
  useStats,
  useSyncVaultNow,
  useUsers,
  useVaultSync,
} from "@/entities/project";
import { useSession } from "@/entities/session";
import { ContextSearch } from "@/widgets/topbar";
import { matchesSearch, settingsTabHref } from "@/shared/lib";
import { request } from "@/shared/api";
import { BarChart, ConfirmDialog, Icon, Menu, useAct, useToast } from "@/shared/ui";
import { McpAdminSection } from "./McpAdmin.tsx";
import { FirstAdmin } from "./ProjectSettings.tsx";
import { SettingsTabs, useSettingsTab } from "./SettingsTabs.tsx";
import { SystemOneSettings } from "./SystemOneSettings.tsx";
import { Costs } from "./ServerCosts.tsx";
import "@/shared/ui/settings.css";
import "@/shared/ui/project.css";

export function ServerSettingsPage() {
  const session = useSession().data;
  const { section = "health" } = useParams();
  const local = session?.mode === "local";
  const tab = useSettingsTab("server", section);
  if (section === "tools" && (tab === "agents" || tab === "health" || tab === "vault"))
    return <Navigate to={settingsTabHref("server", "tools", tab)} replace />;
  return (
    <main className="main">
      <SettingsTabs scope="server" section={section === "health" || section === "vault" ? "tools" : section} current={section === "health" || section === "vault" ? section : undefined} />
      <div className="scroll">
        {section === "projects" ? (
          <Projects current={session?.project} tab={tab} />
        ) : section === "accounts" ? (
          local ? <div className="st-body"><FirstAdmin /></div> : <Accounts me={session?.user.login} tab={tab} />
        ) : section === "providers" ? (
          <ProvidersAdminSection />
        ) : section === "tools" ? (
          <><div hidden={tab !== "jev"}><SystemOneSettings /></div><div hidden={tab !== "mcp"}><McpAdminSection /></div></>
        ) : section === "vault" ? (
          <Knowledge />
        ) : (
          <Health />
        )}
      </div>
    </main>
  );
}

/** The server-wide work and cost statistics, available to server administrators. */
export function ServerStatisticsPage() {
  return (
    <main className="main">
      <header className="topbar">
        <h1>Статистика</h1>
      </header>
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="scroll">
        <Statistics />
      </div>
    </main>
  );
}

const LEVEL_NAME: Record<DoctorCheck["level"], string> = { ok: "в порядке", warn: "предупреждение", fail: "проблема" };

/** Whether the server is ready (`genie doctor`): what needs attention first, what works folded; then the week in numbers. */
function Health() {
  const doctor = useDoctor(true);
  const checks = doctor.data?.checks ?? [];
  const summary = doctorSummary(checks);
  const order: Record<DoctorCheck["level"], number> = { fail: 0, warn: 1, ok: 2 };
  const attention = checks.filter((c) => c.level !== "ok").sort((a, b) => order[a.level] - order[b.level]);
  const fine = checks.filter((c) => c.level === "ok");
  return (
    <div className="st-body sv-wide">
      <div className="st-head">
        <h2>Состояние</h2>
      </div>
      <section className="st-card">
        {doctor.isPending ? (
          <div className="st-card-body muted">Проверяем сервер…</div>
        ) : doctor.isError ? (
          <div className="st-card-body muted">Проверка не удалась: {doctor.error.message}</div>
        ) : (
          <>
            <div className="sv-verdict">
              <span className={`sv-mark ${summary.level}`} aria-hidden="true">
                {summary.level === "ok" ? <Icon.check size={13} /> : "!"}
              </span>
              <span className="sv-verdict-text">
                <b>{summary.text}</b>
                <span>Проверено {new Date(doctor.dataUpdatedAt).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" })}</span>
              </span>
              <button type="button" className="btn" disabled={doctor.isFetching} onClick={() => void doctor.refetch()}>
                <Icon.restart size={12} />
                Проверить снова
              </button>
            </div>
            {attention.length > 0 && <Checks list={attention} />}
            {fine.length > 0 && (
              <details className="sv-fold">
                <summary>В порядке: {fine.length}</summary>
                <Checks list={fine} />
              </details>
            )}
          </>
        )}
      </section>
      <Week />
    </div>
  );
}

function Checks({ list }: { list: DoctorCheck[] }) {
  return (
    <ul className="sv-checks">
      {list.map((c, i) => (
        <li key={i} className={c.level}>
          <span className="lv" aria-label={LEVEL_NAME[c.level]} />
          <span className="area">{DOCTOR_AREA[c.area] ?? c.area}</span>
          <span className="txt">
            {c.text}
            {c.hint && c.level !== "ok" && <span className="hint">{c.hint}</span>}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** The last seven days in four numbers; the details are on the statistics page. */
function Week() {
  const stats = useStats(7);
  const t = statsTotals(stats.data?.projects ?? []);
  return (
    <section className="sv-section">
      <div className="sv-section-head">
        <h3>Последние 7 дней</h3>
        <span className="muted">все проекты</span>
        <Link to="/statistics" className="sv-more">
          Вся статистика
          <Icon.chevron size={12} />
        </Link>
      </div>
      {stats.isError ? (
        <p className="muted">{stats.error.message}</p>
      ) : (
        <div className="sv-tiles">
          <Tile label="Готово задач" value={t.done} sub={`из ${t.created} новых`} />
          <Tile label="Открыто сейчас" value={t.open} sub={t.openProjects ? `в проектах: ${t.openProjects}` : "открытых задач нет"} />
          <Tile label="Вопросы агентов людям" value={t.decisions} sub={t.decisions ? "время ответа — на странице «Статистика»" : "агенты ничего не спрашивали"} />
          <Tile label="Сбои агентов" value={t.runsFailed} sub={`из ${t.runs} запусков`} bad={t.runsFailed > 0} />
        </div>
      )}
    </section>
  );
}

function Tile({ label, value, sub, bad }: { label: string; value: number | string; sub: string; bad?: boolean }) {
  return (
    <div className="sv-tile">
      <span className="lbl">{label}</span>
      <b className={bad ? "bad" : undefined}>{value}</b>
      <span className="sub">{sub}</span>
    </div>
  );
}

/** How the work went over a week or a month (`genie stats`): numbers, two charts by day, then per project. */
function Statistics() {
  const [days, setDays] = useState(7);
  const [only, setOnly] = useState("");
  const [view, setView] = useState<"work" | "cost" | "factory">("work");
  const stats = useStats(days);
  const [params] = useSearchParams();
  const all = (stats.data?.projects ?? []).filter((p) => matchesSearch(params.get("q") ?? "", p.project, p.name));
  const list = only ? all.filter((p) => p.project === only) : all;
  const t = statsTotals(list);
  const series = sumDays(list);
  return (
    <div className="st-body sv-stats">
      <div className="sv-top">
        <div className="seg" role="group" aria-label="Что показать">
          {(["work", "cost", "factory"] as const).map((v) => (
            <button key={v} type="button" className={v === view ? "on" : ""} aria-pressed={v === view} onClick={() => setView(v)}>
              {v === "work" ? "Работа" : v === "cost" ? "Расходы" : "Фабрика"}
            </button>
          ))}
        </div>
        <label className="sr-only" htmlFor="sv-project">
          Проект
        </label>
        <select id="sv-project" className="st-input sv-select" value={only} onChange={(e) => setOnly(e.target.value)}>
          <option value="">Все проекты</option>
          {all.map((p) => (
            <option key={p.project} value={p.project}>
              {p.name}
            </option>
          ))}
        </select>
        <div className="seg" role="group" aria-label="Период">
          {[7, 30].map((d) => (
            <button key={d} type="button" className={d === days ? "on" : ""} aria-pressed={d === days} onClick={() => setDays(d)}>
              {d === 7 ? "Неделя" : "Месяц"}
            </button>
          ))}
        </div>
      </div>
      {stats.isPending ? (
        <p className="muted">Загрузка…</p>
      ) : stats.isError ? (
        <p className="muted">{stats.error.message}</p>
      ) : view === "factory" ? (
        <FactoryAnalytics list={list} />
      ) : view === "cost" ? (
        <Costs list={list} days={days} />
      ) : (
        <>
          <div className="sv-tiles">
            <Tile label="Готово задач" value={t.done} sub={`создано ${t.created}, людьми ${t.createdByPeople}`} />
            <Tile label="Открыто сейчас" value={t.open} sub="не готовы и не отменены" />
            <Tile label="Вопросы агентов людям" value={t.decisions} sub={list.length === 1 && t.decisions ? `ответ за ${hoursText(list[0].answerHoursMedian)} (медиана)` : "время ответа — в таблице ниже"} />
            <Tile label="Возвраты с ревью" value={t.returns} sub="раз работу вернули на доработку" />
          </div>
          <div className="sv-charts">
            <BarChart
              title="Задачи по дням"
              days={series}
              series={[
                { key: "created", label: "создано", color: "var(--sv-created)" },
                { key: "done", label: "готово", color: "var(--sv-done)" },
              ]}
            />
            <BarChart
              title="Запуски агентов"
              days={series}
              stacked
              series={[
                { key: "ok", label: "без сбоя", color: "var(--sv-ok)", value: (d) => d.runs - d.runsFailed },
                { key: "runsFailed", label: "со сбоем", color: "var(--sv-failed)" },
              ]}
            />
          </div>
          <ProjectTable list={list} />
        </>
      )}
    </div>
  );
}

/** Per project: the main numbers in a row, the rest under a disclosure. */
function ProjectTable({ list }: { list: ProjectStats[] }) {
  return (
    <section className="st-card">
      <div className="sv-table">
        <div className="tr th">
          <span>Проект</span>
          <span>Готово / создано</span>
          <span>Открыто</span>
          <span>До готовности</span>
          <span>Ответ людей</span>
          <span>Запуски</span>
        </div>
        {list.map((p) => (
          <div key={p.project} className="tr">
            <span className="nm">
              <b>{p.name}</b>
              <span className="muted">{p.people.length ? p.people.join(", ") : "людей не было"}</span>
            </span>
            <span data-l="Готово / создано">
              {p.done} / {p.created}
            </span>
            <span data-l="Открыто">{p.open}</span>
            <span data-l="До готовности" title={`90% — до ${hoursText(p.cycleHoursP90)}`}>
              {hoursText(p.cycleHoursMedian)}
            </span>
            <span data-l="Ответ людей">{p.decisions ? hoursText(p.answerHoursMedian) : "—"}</span>
            <span data-l="Запуски">
              {p.runs}
              {p.runsFailed > 0 && <span className="pj-bad"> · сбоев {p.runsFailed}</span>}
            </span>
          </div>
        ))}
      </div>
      <details className="sv-fold">
        <summary>Ещё: комментарии, MCP, задания автоматизаций, база знаний</summary>
        <dl className="st-facts sv-more-facts">
          {list.map((p) => (
            <div key={p.project}>
              <dt>{p.name}</dt>
              <dd>
                Комментарии: людей {p.commentsByPeople}, агентов {p.commentsByAgents}. Вызовов MCP {p.mcpCalls}. Заданий {p.jobs}
                {p.jobsFailed ? `, со сбоем ${p.jobsFailed}` : ""}. Предложений в базу знаний {p.proposals}: принято {p.proposalsApproved}, отклонено {p.proposalsRejected}. 90% задач готовы за{" "}
                {hoursText(p.cycleHoursP90)}.
              </dd>
            </div>
          ))}
        </dl>
      </details>
    </section>
  );
}

/** The vault's git remote: where people open it in Obsidian, and how the last sync went. */
function Knowledge() {
  const sync = useVaultSync(true);
  const now = useSyncVaultNow();
  const act = useAct();
  const s = sync.data;
  const last = s?.last;
  return (
    <div className="st-body">
      <div className="st-head">
        <h2>База знаний</h2>
      </div>
      {!s ? (
        <p className="muted">{sync.isPending ? "Загрузка…" : sync.error?.message}</p>
      ) : !s.remote ? (
        <section className="st-card">
          <div className="st-card-body">
            <p className="sv-p">
              База знаний хранится только на этом сервере. Чтобы открыть её в Obsidian и держать ещё одну копию, укажите git-репозиторий в <code>vault.remote</code> в
              config.json: сервер будет забирать и отправлять правки каждые {s.every} с.
            </p>
          </div>
        </section>
      ) : (
        <>
          <section className="st-card">
            <div className="sv-verdict">
              <span className={`sv-dot ${!last?.at ? "" : last.ok ? "ok" : "fail"}`} aria-hidden="true" />
              <span className="sv-verdict-text">
                <b>{!last?.at ? "Синхронизации ещё не было" : last.ok ? `Синхронизирована ${new Date(last.at).toLocaleString("ru-RU")}` : "Синхронизация не удалась"}</b>
                <span className={last?.at && !last.ok ? "pj-bad" : undefined}>
                  {!last?.at
                    ? `Сервер синхронизирует каждые ${s.every} с.`
                    : last.ok
                      ? `Получено коммитов: ${last.pulled}, отправлено: ${last.pushed}. Сервер синхронизирует каждые ${s.every} с.`
                      : `${new Date(last.at).toLocaleString("ru-RU")}: ${last.error}`}
                </span>
              </span>
              <button type="button" className="btn" disabled={now.isPending} onClick={() => void act(() => now.mutateAsync(), "База знаний синхронизирована")}>
                <Icon.restart size={12} />
                Синхронизировать сейчас
              </button>
            </div>
            <div className="st-row">
              <div className="lbl">
                <b>Репозиторий</b>
              </div>
              <div className="ctl">
                <span>
                  <code className="sv-mono">{s.remote}</code>
                  {last?.branch && <span className="muted"> · {last.branch}</span>}
                </span>
              </div>
            </div>
            {last && last.conflicts.length > 0 && (
              <div className="st-row">
                <div className="lbl">
                  <b>Пересечения</b>
                  <span>Здесь осталась версия сервера, версия из репозитория лежит рядом копией .conflict.</span>
                </div>
                <div className="ctl">
                  {last.conflicts.map((c) => (
                    <code key={c} className="sv-mono">
                      {c}
                    </code>
                  ))}
                </div>
              </div>
            )}
          </section>
          <details className="st-card">
            <summary>Как открыть в Obsidian</summary>
            <p className="sv-p sv-pad">
              Склонируйте репозиторий и откройте папку как хранилище Obsidian; правки отправляйте в git, например плагином Obsidian Git. Правки разных строк сливаются
              сами; если одни и те же строки изменили и там, и на сервере, остаётся версия сервера, а версия из репозитория ложится рядом копией <code>.conflict</code> —
              ничего не теряется.
            </p>
          </details>
        </>
      )}
    </div>
  );
}

function Projects({ current, tab }: { current?: string; tab: string }) {
  const projects = useProjects();
  const qc = useQueryClient();
  const act = useAct();
  return (
    <div className="st-body">
      <div className="st-col" hidden={tab !== "list"}>
        <div className="st-head">
          <h2>Проекты</h2>
        </div>
        <section className="st-card">
          <ul className="sv-list">
            {(projects.data ?? []).map((p) => (
              <li key={p.slug}>
                <span className="sv-initial" aria-hidden="true">
                  {p.name.slice(0, 1).toUpperCase()}
                </span>
                <span className="who">
                  <b>{p.name}</b>
                  <span className="muted">
                    <span className="sv-mono">{p.slug}</span> · {p.repo ? <span className="sv-mono">{p.repo}</span> : "без кода"} ·{" "}
                    {AUTONOMY.find((a) => a.id === p.autonomy)?.name ?? p.autonomy}
                  </span>
                </span>
                {p.slug === current ? (
                  <span className="sv-badge">открыт</span>
                ) : (
                  <button
                    type="button"
                    className="btn"
                    onClick={() =>
                      void act(async () => {
                        await request("POST", "/api/session/project", { project: p.slug });
                        await qc.resetQueries();
                      }, `Открыт проект ${p.name}`)
                    }
                  >
                    Открыть
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      </div>
      <aside className="st-card" aria-label="Новый проект" hidden={tab !== "create"}>
        <div className="st-card-head">
          <h3>Новый проект</h3>
        </div>
        <div className="st-card-body">
          <NewProjectForm folded />
        </div>
      </aside>
    </div>
  );
}

/** The first screen of a server without projects, for its administrator. */
export function FirstProjectPage() {
  return (
    <div className="auth-page">
      <div className="auth-card pj-first">
        <div className="auth-head">
          <span className="logo-mark lg">
            <Icon.mark size={34} />
          </span>
          <h1>Первый проект</h1>
          <p className="auth-sub">Создайте первый проект.</p>
        </div>
        <NewProjectForm />
      </div>
    </div>
  );
}

/** A new project; the server switches to it. `folded` puts the repository and the prefix under «Дополнительно». */
export function NewProjectForm({ onCreated, folded }: { onCreated?: () => void; folded?: boolean }) {
  const create = useCreateProject();
  const qc = useQueryClient();
  const act = useAct();
  const [form, setForm] = useState({ slug: "", name: "", repo: "", prefix: "" });
  const extra = (
    <>
      <label className="field">
        Репозиторий кода на этой машине
        <input className="mono" value={form.repo} placeholder="/home/team/code/shop — пусто, если проект без кода" onChange={(e) => setForm({ ...form, repo: e.target.value })} />
        <span className="pj-hint">Задачи из .genie/ будут импортированы.</span>
      </label>
      <label className="field">
        Префикс задач
        <input value={form.prefix} placeholder="по коду проекта, например SHOP" onChange={(e) => setForm({ ...form, prefix: e.target.value.toUpperCase() })} />
      </label>
    </>
  );
  const slug = (
    <label className="field">
      Код (латиница)
      <input value={form.slug} pattern="[a-z0-9][a-z0-9-]*" title="строчные латинские буквы, цифры и дефис" placeholder="shop" onChange={(e) => setForm({ ...form, slug: e.target.value })} required />
      {folded && <span className="pj-hint">В адресах и командах genie; потом не меняется.</span>}
    </label>
  );
  const name = (
    <label className="field">
      Название
      <input value={form.name} placeholder="Магазин" onChange={(e) => setForm({ ...form, name: e.target.value })} />
    </label>
  );
  return (
    <form
      className="pj-form"
      onSubmit={(e) => {
        e.preventDefault();
        void act(async () => {
          const p = await create.mutateAsync({ slug: form.slug.trim(), name: form.name.trim(), repo: form.repo.trim() || undefined, prefix: form.prefix.trim() || undefined });
          await request("POST", "/api/session/project", { project: p.slug });
          await qc.resetQueries();
          setForm({ slug: "", name: "", repo: "", prefix: "" });
          onCreated?.();
        }, "Проект создан");
      }}
    >
      {folded ? (
        <>
          {name}
          {slug}
          <details className="sv-extra">
            <summary>Дополнительно</summary>
            <div className="sv-extra-body">{extra}</div>
          </details>
        </>
      ) : (
        <>
          <div className="pj-pair">
            {slug}
            {name}
          </div>
          {extra}
        </>
      )}
      <button className="btn primary" disabled={create.isPending}>
        Создать проект
      </button>
    </form>
  );
}

function Accounts({ me, tab }: { me?: string; tab: string }) {
  const users = useUsers();
  const projects = useProjects();
  const member = useSetMember();
  const remove = useDeleteUser();
  const toast = useToast();
  const [removing, setRemoving] = useState<Person>();
  const [credentials, setCredentials] = useState<{ login: string; password: string }>();
  const [assignment, setAssignment] = useState<{ user: number; slug: string }>({ user: 0, slug: "" });
  const patch = usePatchUser();
  const create = useCreateUser();
  const act = useAct();
  const [form, setForm] = useState({ login: "", name: "", password: "", isAdmin: false });
  const list = users.data ?? [];
  return (
    <div className="st-body">
      <div className="st-col" hidden={tab !== "list"}>
        <div className="st-head">
          <h2>Учётные записи</h2>
          <p>В проект можно пригласить по ссылке со страницы проекта.</p>
        </div>
        <section className="st-card">
          <ul className="sv-list">
            {list.map((u) => (
              <li key={u.id} className={u.disabled ? "off" : undefined}>
                <PersonAvatar login={u.login} name={u.name} src={u.avatar} size="md" prefix="" />
                <span className="who">
                  <b>{personName(u)}</b>
                  <span className="muted">
                    @{u.login}
                    {u.email ? ` · ${u.email}` : ""} · с {new Date(u.created).toLocaleDateString("ru-RU")}
                  </span>
                </span>
                {u.login === me ? (
                  <span className="pj-role">{u.isAdmin ? "администратор · это вы" : "это вы"}</span>
                ) : u.disabled ? (
                  <span className="sv-badge off">отключена</span>
                ) : (
                  <select
                    aria-label={`Права ${u.login}`}
                    value={u.isAdmin ? "admin" : "user"}
                    onChange={(e) => {
                      const isAdmin = e.target.value === "admin";
                      void act(() => patch.mutateAsync({ id: u.id, patch: { isAdmin } }), isAdmin ? `${personName(u)} — администратор сервера` : `${personName(u)} больше не администратор`);
                    }}
                  >
                    <option value="user">пользователь</option>
                    <option value="admin">администратор</option>
                  </select>
                )}
                {u.login === me ? (
                  <span className="sv-menu-gap" />
                ) : (
                  <Menu label={`Ещё для ${u.login}`}>
                    {(close) => (
                      <><button type="button" role="menuitem" onClick={() => { close(); setRemoving(u); }}>Удалить учётную запись</button>
                      <button
                        type="button"
                        role="menuitem"
                        className={u.disabled ? undefined : "danger"}
                        onClick={() => {
                          close();
                          const disabled = !u.disabled;
                          void act(() => patch.mutateAsync({ id: u.id, patch: { disabled } }), disabled ? `${personName(u)} отключён` : `${personName(u)} снова может войти`);
                        }}
                      >
                        {u.disabled ? "Включить учётную запись" : "Отключить учётную запись"}
                      </button></>
                    )}
                  </Menu>
                )}
              </li>
            ))}
          </ul>
        </section>
        <details className="st-card">
          <summary>Администратор сервера и отключение</summary>
          <dl className="st-roles sv-two">
            <div>
              <dt>Администратор сервера</dt>
              <dd>работает во всех проектах как владелец; роли агентов, учётные записи, эта страница</dd>
            </div>
            <div>
              <dt>Отключённая запись</dt>
              <dd>не может войти; её сессии и токены перестают работать. Включается снова в меню «⋯»</dd>
            </div>
          </dl>
        </details>
      </div>
      <div hidden={tab !== "access"}>        <form className="st-card" onSubmit={(e) => { e.preventDefault(); void act(() => member.mutateAsync({ slug: assignment.slug, user: assignment.user, role: "admin" }), "Администратор проекта назначен"); }}>
          <div className="st-card-head"><h3>Назначить администратора проекта</h3><p>Роль назначается для выбранного проекта.</p></div>
          <div className="st-card-body pj-form">
            <label className="field">Учётная запись<select required value={assignment.user || ""} onChange={(e) => setAssignment({ ...assignment, user: Number(e.target.value) })}><option value="">Выберите пользователя</option>{list.filter((u) => !u.disabled && !u.isAdmin).map((u) => <option key={u.id} value={u.id}>{personName(u)}</option>)}</select></label>
            <label className="field">Проект<select required value={assignment.slug} onChange={(e) => setAssignment({ ...assignment, slug: e.target.value })}><option value="">Выберите проект</option>{projects.data?.map((p) => <option key={p.slug} value={p.slug}>{p.name}</option>)}</select></label>
            <button className="btn" disabled={!assignment.user || !assignment.slug || member.isPending}>Назначить</button>
          </div>
        </form>
</div>
      <aside className="st-card" aria-label="Новая учётная запись" hidden={tab !== "create"}>
        <div className="st-card-head">
          <h3>Новая учётная запись</h3>
        </div>
        <form
          className="pj-form st-card-body"
          onSubmit={(e) => {
            e.preventDefault();
            void act(async () => {
              await create.mutateAsync({ login: form.login.trim(), name: form.name.trim(), password: form.password, isAdmin: form.isAdmin });
              setCredentials({ login: form.login.trim(), password: form.password });
              setForm({ login: "", name: "", password: "", isAdmin: false });
            }, "Учётная запись создана: добавьте человека в проект на странице проекта");
          }}
        >
          <label className="field">
            Логин (латиница)
            <input autoComplete="off" value={form.login} onChange={(e) => setForm({ ...form, login: e.target.value })} required />
          </label>
          <label className="field">
            Имя
            <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
          </label>
          <label className="field">
            Пароль
            <input type="password" autoComplete="new-password" minLength={8} value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} required />
            <span className="pj-hint">Не короче 8 символов.</span>
          </label>
          <label className="pj-check">
            <input type="checkbox" checked={form.isAdmin} onChange={(e) => setForm({ ...form, isAdmin: e.target.checked })} />
            администратор сервера
          </label>
          <button className="btn primary" disabled={create.isPending}>
            Создать
          </button>
          <span className="pj-hint">Потом добавьте человека в проект на странице проекта.</span>
        </form>
        {credentials && <div className="st-card-body"><p>Создана учётная запись @{credentials.login}.</p><button type="button" className="btn" onClick={() => void act(async () => {
          await navigator.clipboard.writeText(`Адрес: ${location.origin}\nЛогин: ${credentials.login}\nПароль: ${credentials.password}`);
          toast("Данные скопированы");
        })}>Скопировать данные для входа</button><button type="button" className="btn ghost" onClick={() => setCredentials(undefined)}>Скрыть данные</button></div>}
      </aside>
      {removing && <ConfirmDialog title="Удалить учётную запись?" confirmLabel="Удалить" danger busy={remove.isPending} onClose={() => setRemoving(undefined)} onConfirm={() => void act(() => remove.mutateAsync(removing.id), "Учётная запись удалена").then((ok) => { if (ok) setRemoving(undefined); })}>
        Учётная запись @{removing.login}, её доступ к проектам, сессии и токены будут удалены. История задач сохранится.
      </ConfirmDialog>}
    </div>
  );
}
