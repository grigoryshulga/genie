// The project's settings, a tab each: general (name, where results live), how agents
// work (the orchestrator, the default integration), repositories, people and invitations.
// Admins of the project change them, the rest see them.

import { useState } from "react";
import { Link, NavLink, useParams } from "react-router";
import {
  AUTONOMY,
  type Membership,
  PersonAvatar,
  personName,
  PROJECT_ROLE_HINT,
  PROJECT_ROLE_NAME,
  type ProjectInfo,
  type ProjectRole,
  useInvite,
  useMembers,
  usePatchProject,
  useProjects,
  useRemoveMember,
  useSetMember,
  useUsers,
} from "@/entities/project";
import { useRepos } from "@/entities/repo";
import { useSession } from "@/entities/session";
import { request } from "@/shared/api";
import { ConfirmDialog, Icon, useAct, useToast } from "@/shared/ui";
import { TelegramSettings } from "./TelegramSettings.tsx";
import { RepositoriesSection } from "./RepositoriesSection.tsx";
import { JevSettings } from "./JevSettings.tsx";
import "@/shared/ui/settings.css";
import "@/shared/ui/project.css";

export const ROLES: ProjectRole[] = ["viewer", "member", "admin", "owner"];

export function ProjectSettingsPage() {
  const session = useSession().data;
  const projects = useProjects();
  const { section = "general" } = useParams();
  const project = projects.data?.find((p) => p.slug === session?.project);
  const local = session?.mode === "local";
  const admin = !!project && (project.role === "admin" || project.role === "owner");
  const repos = useRepos().data?.length ?? 0;
  const members = useMembers(project && !local ? project.slug : "").data?.length ?? 0;
  const tabs = [
    { id: "general", title: "Общие" },
    { id: "agents", title: "Работа агентов" },
    { id: "decisions", title: "Решения с Jev" },
    { id: "repos", title: "Репозитории", n: repos },
    { id: "people", title: "Люди", n: members },
    { id: "telegram", title: "Telegram" },
  ];
  return (
    <main className="main">
      <header className="topbar">
        <h1>Настройки проекта</h1>
        {project && (
          <span className="sub">
            {project.name} · {PROJECT_ROLE_NAME[project.role]}
          </span>
        )}
        <nav className="st-tabs" aria-label="Разделы настроек проекта">
          {tabs.map((t) => (
            <NavLink key={t.id} to={`/settings/project/${t.id}`} end className={({ isActive }) => (isActive ? "on" : undefined)}>
              {t.title}
              {!!t.n && <span className="n">{t.n}</span>}
            </NavLink>
          ))}
        </nav>
      </header>
      <div className="scroll">
        {!project ? (
          <div className="empty">{projects.isPending ? "Загрузка…" : (projects.error?.message ?? "Проект не выбран")}</div>
        ) : section === "telegram" ? (
          <TelegramSettings key={project.slug} project={project.slug} admin={admin} />
        ) : section === "agents" ? (
          <AgentsTab key={`${project.slug}:${project.integration}`} project={project} admin={admin} />
        ) : section === "decisions" ? (
          <JevSettings key={project.slug} slug={project.slug} admin={admin} />
        ) : section === "repos" ? (
          <div className="st-body">
            <RepositoriesSection admin={admin} />
          </div>
        ) : section === "people" ? (
          local ? (
            <div className="st-body">
              <FirstAdmin />
            </div>
          ) : (
            <div className={`st-body${admin ? " split" : ""}`}>
              <div className="st-col">
                <div className="st-head">
                  <h2>Люди</h2>
                  <p>Администраторы сервера имеют права владельца во всех проектах.</p>
                </div>
                <People slug={project.slug} admin={admin} me={session?.user.login} />
                <details className="st-card">
                  <summary>Что может каждая роль</summary>
                  <dl className="st-roles">
                    {ROLES.map((r) => (
                      <div key={r}>
                        <dt>{PROJECT_ROLE_NAME[r][0].toUpperCase() + PROJECT_ROLE_NAME[r].slice(1)}</dt>
                        <dd>{PROJECT_ROLE_HINT[r]}</dd>
                      </div>
                    ))}
                  </dl>
                </details>
              </div>
              {admin && <Invite slug={project.slug} />}
            </div>
          )
        ) : (
          <General key={`${project.slug}:${project.name}`} project={project} admin={admin} />
        )}
      </div>
    </main>
  );
}

function General({ project, admin }: { project: ProjectInfo; admin: boolean }) {
  const [name, setName] = useState(project.name);
  const patch = usePatchProject();
  const repos = useRepos().data ?? [];
  const act = useAct();
  const toast = useToast();
  const dirty = name.trim() !== project.name;
  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Общие</h2>
        {!admin && <p>Изменять настройки могут администраторы и владельцы проекта.</p>}
      </div>
      <form
        className="st-card"
        onSubmit={(e) => {
          e.preventDefault();
          void act(() => patch.mutateAsync({ slug: project.slug, patch: { name: name.trim() } }), "Название сохранено");
        }}
      >
        <div className="st-card-head">
          <h3>Проект</h3>
        </div>
        <div className="st-row">
          <label className="lbl" htmlFor="pj-name">
            <b>Название</b>
            <span>Видно в переключателе проектов, письмах и уведомлениях.</span>
          </label>
          <div className="ctl">
            {admin ? <input id="pj-name" className="st-input" value={name} onChange={(e) => setName(e.target.value)} required /> : <span id="pj-name">{project.name}</span>}
          </div>
        </div>
        <div className="st-row">
          <div className="lbl">
            <b>Код</b>
            <span>В адресах, командах genie и API. Задаётся при создании и не меняется.</span>
          </div>
          <div className="ctl" style={{ flexDirection: "row", alignItems: "center" }}>
            <code className="st-code">{project.slug}</code>
            <button
              type="button"
              className="btn ghost sm"
              onClick={() =>
                void navigator.clipboard?.writeText(project.slug).then(
                  () => toast("Код скопирован"),
                  () => toast("Скопируйте вручную", "error"),
                )
              }
            >
              Скопировать
            </button>
          </div>
        </div>
        {admin && (
          <div className="st-foot">
            <span className="grow" />
            <button type="button" className="btn ghost" disabled={!dirty} onClick={() => setName(project.name)}>
              Отменить
            </button>
            <button className="btn primary" disabled={!dirty || !name.trim() || patch.isPending}>
              Сохранить
            </button>
          </div>
        )}
      </form>

      <section className="st-card">
        <div className="st-card-head">
          <h3>Где живут результаты</h3>
        </div>
        <div className="st-row">
          <div className="lbl">
            <b>Документация</b>
            <span>Раздел базы знаний проекта. Агенты читают его перед работой и предлагают правки.</span>
          </div>
          <div className="ctl" style={{ flexDirection: "row", alignItems: "center" }}>
            <code className="mono">{project.space}/</code>
            <Link to="/docs" style={{ marginLeft: "auto", fontSize: 12 }}>
              Открыть
            </Link>
          </div>
        </div>
        <div className="st-row">
          <div className="lbl">
            <b>Репозитории</b>
            <span>Код проекта и что агентам разрешено в нём делать.</span>
          </div>
          <div className="ctl">
            {repos.length ? (
              repos.map((r) => (
                <span key={r.name} style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                  <span className="mono">{r.name}</span>
                  <span className="pill">{r.access === "write" ? "запись" : "чтение"}</span>
                </span>
              ))
            ) : (
              <span className="muted">{project.repo ? <span className="mono">{project.repo}</span> : "без кода: результат — артефакты задач и страницы документации"}</span>
            )}
            <Link to="/settings/project/repos" style={{ fontSize: 12 }}>
              {admin ? "Настроить во вкладке «Репозитории»" : "Подробнее во вкладке «Репозитории»"}
            </Link>
          </div>
        </div>
      </section>
    </div>
  );
}

/** Who leads tasks and who closes accepted ones, in each mode of the orchestrator. */
const MODE_WHO: Record<string, { leads: string; closes: string }> = {
  autonomous: { leads: "оркестратор", closes: "оркестратор" },
  assisted: { leads: "оркестратор", closes: "люди" },
  manual: { leads: "люди", closes: "люди" },
};

function AgentsTab({ project, admin }: { project: ProjectInfo; admin: boolean }) {
  const [integration, setIntegration] = useState(project.integration);
  const repos = useRepos().data ?? [];
  const patch = usePatchProject();
  const act = useAct();
  const save = (p: Parameters<typeof patch.mutateAsync>[0]["patch"], ok: string) => act(() => patch.mutateAsync({ slug: project.slug, patch: p }), ok);
  const dirty = integration.trim() !== project.integration;
  const code = !!project.repo || repos.length > 0;
  const examples = [
    ...(code
      ? [
          { title: "Запрос в main", text: "Исполнитель открывает запрос на слияние из ветки genie/<задача> в main, ревьюер проверяет, владелец сливает." },
          { title: "Ветка, сливает владелец", text: "Владелец смотрит ветку genie/<задача> и сам сливает её в main." },
        ]
      : []),
    { title: "Страница документации", text: "Результат — страница документации и артефакт в задаче." },
  ];
  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Работа агентов</h2>
      </div>
      <section className="st-card">
        <div className="st-card-head">
          <h3>Оркестратор</h3>
          <p>{admin ? "Применяется сразу, к задачам в работе тоже." : "Режим выбирают админы и владельцы проекта."}</p>
        </div>
        <fieldset className="st-modes" disabled={!admin || patch.isPending}>
          <legend className="sr-only">Режим оркестратора</legend>
          {AUTONOMY.map((a) => {
            const who = MODE_WHO[a.id];
            return (
              <label key={a.id} className={`st-mode${project.autonomy === a.id ? " on" : ""}`}>
                <span className="top">
                  <input type="radio" name="autonomy" checked={project.autonomy === a.id} onChange={() => void save({ autonomy: a.id }, `Оркестратор: ${a.name.toLowerCase()}`)} />
                  {a.name}
                </span>
                <span className="hint">{a.hint[0].toUpperCase() + a.hint.slice(1)}.</span>
                {who && (
                  <span className="who">
                    <span>
                      Ведёт задачи: <b className={who.leads === "люди" ? undefined : "agent"}>{who.leads}</b>
                    </span>
                    <span>
                      Закрывает принятые: <b className={who.closes === "люди" ? undefined : "agent"}>{who.closes}</b>
                    </span>
                  </span>
                )}
              </label>
            );
          })}
        </fieldset>
      </section>

      <form
        className="st-card"
        onSubmit={(e) => {
          e.preventDefault();
          void save({ integration: integration.trim() }, "Интеграция по умолчанию сохранена");
        }}
      >
        <div className="st-card-head">
          <h3>Интеграция по умолчанию</h3>
          <p>Команда получает это вместе с каждой задачей, если в самой задаче не договорились иначе.</p>
        </div>
        <div className="st-row">
          <label className="lbl" htmlFor="pj-integration">
            <b>Как результат попадает в систему</b>
            <span>Одно-два предложения обычным языком.</span>
          </label>
          <div className="ctl">
            {admin ? (
              <>
                <textarea
                  id="pj-integration"
                  rows={3}
                  value={integration}
                  placeholder={code ? "Например: владелец смотрит ветку genie/<задача> и сливает её в main" : "Например: результат — страница документации и артефакт в задаче"}
                  onChange={(e) => setIntegration(e.target.value)}
                />
                <span className="st-chips">
                  Шаблоны:
                  {examples.map((x) => (
                    <button key={x.title} type="button" onClick={() => setIntegration(x.text)}>
                      {x.title}
                    </button>
                  ))}
                </span>
              </>
            ) : (
              <span id="pj-integration" className={project.integration ? undefined : "muted"}>
                {project.integration || "не задана: команда договаривается о ней в каждой задаче"}
              </span>
            )}
          </div>
        </div>
        {admin && (
          <div className="st-foot">
            <span className="grow">{dirty ? "Есть несохранённые изменения" : ""}</span>
            <button type="button" className="btn ghost" disabled={!dirty} onClick={() => setIntegration(project.integration)}>
              Отменить
            </button>
            <button className="btn primary" disabled={!dirty || patch.isPending}>
              Сохранить
            </button>
          </div>
        )}
      </form>
    </div>
  );
}

function People({ slug, admin, me }: { slug: string; admin: boolean; me?: string }) {
  const members = useMembers(slug);
  const users = useUsers(admin);
  const setMember = useSetMember();
  const remove = useRemoveMember();
  const act = useAct();
  const [adding, setAdding] = useState<{ user: string; role: ProjectRole }>({ user: "", role: "member" });
  const [removing, setRemoving] = useState<Membership>();
  const list = members.data ?? [];
  const outside = (users.data ?? []).filter((u) => !u.disabled && !list.some((m) => m.user.id === u.id));
  return (
    <section className="st-card" aria-label="Люди проекта">
      {admin && outside.length > 0 && (
        <form
          className="pj-row st-card-head"
          style={{ flexDirection: "row" }}
          onSubmit={(e) => {
            e.preventDefault();
            const user = outside.find((u) => String(u.id) === adding.user);
            if (!user) return;
            void act(() => setMember.mutateAsync({ slug, user: user.id, role: adding.role }), `${personName(user)} теперь в проекте`).then(
              (ok) => ok && setAdding({ user: "", role: "member" }),
            );
          }}
        >
          <select aria-label="Кого добавить" value={adding.user} onChange={(e) => setAdding({ ...adding, user: e.target.value })} style={{ flex: 1 }}>
            <option value="">Человек с сервера…</option>
            {outside.map((u) => (
              <option key={u.id} value={u.id}>
                {u.name ? `${u.name} (@${u.login})` : `@${u.login}`}
              </option>
            ))}
          </select>
          <select aria-label="Роль нового участника" value={adding.role} onChange={(e) => setAdding({ ...adding, role: e.target.value as ProjectRole })}>
            {ROLES.map((r) => (
              <option key={r} value={r}>
                {PROJECT_ROLE_NAME[r]}
              </option>
            ))}
          </select>
          <button className="btn" disabled={!adding.user || setMember.isPending}>
            Добавить
          </button>
        </form>
      )}
      <div className="st-card-body" style={{ paddingTop: 4, paddingBottom: 4 }}>
      {list.length > 0 && (
        <ul className="pj-people">
          {list.map((m) => (
            <li key={m.user.id} className={m.user.disabled ? "off" : undefined}>
              <PersonAvatar login={m.user.login} name={m.user.name} src={m.user.avatar} size="md" prefix="" />
              <span className="who">
                <b>
                  {personName(m.user)}
                  {m.user.login === me ? " — вы" : ""}
                </b>
                <span className="muted">
                  @{m.user.login}
                  {m.user.email ? ` · ${m.user.email}` : ""}
                  {m.user.isAdmin ? " · администратор сервера" : ""}
                  {m.user.disabled ? " · отключён" : ""}
                </span>
              </span>
              {admin && m.user.login !== me ? (
                <>
                  <select
                    aria-label={`Роль ${m.user.login}`}
                    value={m.role}
                    onChange={(e) => {
                      const role = e.target.value as ProjectRole;
                      void act(() => setMember.mutateAsync({ slug, user: m.user.id, role }), `${personName(m.user)}: ${PROJECT_ROLE_NAME[role]}`);
                    }}
                  >
                    {ROLES.map((r) => (
                      <option key={r} value={r}>
                        {PROJECT_ROLE_NAME[r]}
                      </option>
                    ))}
                  </select>
                  <button type="button" className="icon-btn" aria-label={`Убрать ${m.user.login} из проекта`} title="Убрать из проекта" onClick={() => setRemoving(m)}>
                    <Icon.trash size={13} />
                  </button>
                </>
              ) : (
                <span className="pj-role">{PROJECT_ROLE_NAME[m.role]}</span>
              )}
            </li>
          ))}
        </ul>
      )}
      {!list.length && <p className="muted">{members.isPending ? "Загрузка…" : "В проекте пока никого нет: пригласите людей ссылкой."}</p>}
      </div>
      {removing && (
        <ConfirmDialog
          title="Убрать из проекта?"
          confirmLabel="Убрать"
          danger
          busy={remove.isPending}
          onClose={() => setRemoving(undefined)}
          onConfirm={() =>
            void act(() => remove.mutateAsync({ slug, user: removing.user.id }), `${personName(removing.user)} больше не в проекте`).then(() => setRemoving(undefined))
          }
        >
          {personName(removing.user)} перестанет видеть задачи и документацию проекта. Учётная запись на сервере останется; задачи, где человек ответственный, лучше передать
          другому.
        </ConfirmDialog>
      )}
    </section>
  );
}

function Invite({ slug }: { slug: string }) {
  const invite = useInvite();
  const act = useAct();
  const toast = useToast();
  const [role, setRole] = useState<ProjectRole>("member");
  const [email, setEmail] = useState("");
  const [link, setLink] = useState<string>();
  return (
    <aside className="st-card" aria-label="Пригласить">
      <div className="st-card-head">
        <h3>Пригласить по ссылке</h3>
        <p>Ссылка открывается один раз и действует 7 дней.</p>
      </div>
      <form
        className="st-card-body"
        onSubmit={(e) => {
          e.preventDefault();
          void act(async () => setLink((await invite.mutateAsync({ slug, role, email: email.trim() })).url));
        }}
      >
        <label className="field">
          Роль
          <select value={role} onChange={(e) => setRole(e.target.value as ProjectRole)}>
            {ROLES.map((r) => (
              <option key={r} value={r}>
                {PROJECT_ROLE_NAME[r]}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          Почта, необязательно
          <input type="email" placeholder="на неё придут уведомления" value={email} onChange={(e) => setEmail(e.target.value)} />
        </label>
        <button className="btn primary" style={{ height: 36 }} disabled={invite.isPending}>
          Создать ссылку
        </button>
      {link && (
        <div className="pj-link" style={{ flexDirection: "column", alignItems: "stretch" }}>
          <span className="secret">{link}</span>
          <button
            type="button"
            className="btn"
            onClick={() =>
              void navigator.clipboard?.writeText(link).then(
                () => toast("Ссылка скопирована"),
                () => toast("Скопируйте ссылку вручную", "error"),
              )
            }
          >
            Скопировать
          </button>
        </div>
      )}
        <p className="muted" style={{ fontSize: 12, lineHeight: 1.5, margin: 0 }}>
          Человек сам выбирает логин и пароль и сразу попадает в проект с этой ролью.
        </p>
      </form>
    </aside>
  );
}

/** Local mode: the server has no users yet and trusts this machine. The first account ends that. */
function FirstAdmin() {
  const act = useAct();
  const [form, setForm] = useState({ login: "", name: "", password: "" });
  return (
    <section className="st-card">
      <div className="st-card-head">
        <h3>Первый администратор</h3>
        <p>
          Пользователей пока нет, и сервер открыт без входа — только с этой машины. Создайте учётную запись администратора: после этого вход станет обязательным для всех,
          а остальных людей вы пригласите ссылками.
        </p>
      </div>
      <form
        className="auth-form narrow st-card-body"
        onSubmit={(e) => {
          e.preventDefault();
          void act(async () => {
            await request("POST", "/api/users", { login: form.login.trim(), name: form.name.trim(), password: form.password, isAdmin: true });
            await request("POST", "/api/auth/login", { login: form.login.trim(), password: form.password });
            window.location.assign("/settings/project/people");
          });
        }}
      >
        <label className="field">
          Логин (латиница)
          <input autoComplete="username" value={form.login} onChange={(e) => setForm({ ...form, login: e.target.value })} required />
        </label>
        <label className="field">
          Имя
          <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
        </label>
        <label className="field">
          Пароль (не короче 8 символов)
          <input type="password" autoComplete="new-password" minLength={8} value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} required />
        </label>
        <button className="btn primary">Создать администратора и войти</button>
      </form>
    </section>
  );
}
