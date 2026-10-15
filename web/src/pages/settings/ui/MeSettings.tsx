// «Личное» — the person's own settings, one section each: the account (photo,
// name, login, mail), signing in (password), Telegram, the model keys agents
// use on one's behalf (the LiteLLM proxy and the providers pi supports), and
// personal tokens.

import { useRef, useState } from "react";
import { NavLink, useParams } from "react-router";
import { PersonAvatar, PROJECT_ROLE_NAME, usePatchUser, useSetAvatar } from "@/entities/project";
import { useChannels, useLlmKeys, type LlmProvider } from "@/entities/platform";
import { type SessionUser, useSession } from "@/entities/session";
import { request } from "@/shared/api";
import { timeAgo } from "@/shared/lib";
import { Icon, useRefreshingAct as useAct } from "@/shared/ui";
import { TokensTab } from "./TokensTab.tsx";
import "@/shared/ui/settings.css";

const SECTIONS = [
  { id: "account", title: "Аккаунт" },
  { id: "security", title: "Вход и пароль" },
  { id: "telegram", title: "Telegram" },
  { id: "keys", title: "Провайдеры и ключи" },
  { id: "tokens", title: "Токены CLI и MCP" },
] as const;

export function MeSettingsPage() {
  const session = useSession().data;
  const { section = "account" } = useParams();
  const local = session?.mode === "local";
  return (
    <main className="main">
      <header className="topbar">
        <h1>Личное</h1>
        <nav className="st-tabs" aria-label="Разделы личных настроек">
          {SECTIONS.map((s) => (
            <NavLink key={s.id} to={`/settings/me/${s.id}`} end className={({ isActive }) => (isActive ? "on" : undefined)}>
              {s.title}
            </NavLink>
          ))}
        </nav>
      </header>
      <div className="scroll">
        {!session ? null : local ? (
          <div className="st-body">
            <div className="st-head">
              <h2>Локальный режим</h2>
              <p>
                На сервере пока нет пользователей, и он открыт без входа только с этой машины. Создайте учётную запись администратора в настройках проекта («Настройки» → «Проект» → «Люди») или
              </p>
            </div>
          </div>
        ) : section === "security" ? (
          <SecurityTab />
        ) : section === "telegram" ? (
          <TelegramTab />
        ) : section === "keys" ? (
          <KeysTab />
        ) : section === "tokens" ? (
          <TokensTab />
        ) : (
          <AccountTab key={`${session.user.login}:${session.user.name}:${session.user.email ?? ""}`} user={session.user} />
        )}
      </div>
    </main>
  );
}

function AccountTab({ user }: { user: SessionUser }) {
  const session = useSession().data;
  const patch = usePatchUser();
  const act = useAct();
  const initial = { name: user.name, login: user.login, email: user.email ?? "" };
  const [form, setForm] = useState(initial);
  const login = form.login.trim().toLowerCase();
  const loginOk = /^[a-z0-9._-]+$/.test(login);
  const loginChanged = login !== user.login;
  const dirty = form.name.trim() !== user.name || loginChanged || form.email.trim() !== (user.email ?? "");
  const projects = session?.projects ?? [];
  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Аккаунт</h2>
        <p>Как вас видят люди и агенты в задачах, почте команды и уведомлениях.</p>
      </div>

      <form
        className="st-card"
        onSubmit={(e) => {
          e.preventDefault();
          if (!dirty || !loginOk) return;
          void act(
            () => patch.mutateAsync({ id: user.id, patch: { name: form.name.trim(), login, email: form.email.trim() || null } }),
            loginChanged ? `Теперь вы @${login}` : "Сохранено",
          );
        }}
      >
        <div className="st-row">
          <div className="lbl">
            <b>Фото</b>
            <span>Видно в списке людей, в задачах и в меню слева.</span>
          </div>
          <Photo user={user} />
        </div>
        <div className="st-row">
          <label className="lbl" htmlFor="pf-name">
            <b>Отображаемое имя</b>
            <span>Так вас называют в интерфейсе и письмах агентов.</span>
          </label>
          <div className="ctl">
            <input id="pf-name" className="st-input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} autoComplete="name" />
          </div>
        </div>
        <div className="st-row">
          <label className="lbl" htmlFor="pf-login">
            <b>Логин</b>
            <span>Для входа и упоминаний. Латиница, цифры, точка, дефис, подчёркивание.</span>
          </label>
          <div className="ctl">
            <div className="st-prefixed">
              <span>@</span>
              <input
                id="pf-login"
                value={form.login}
                onChange={(e) => setForm({ ...form, login: e.target.value })}
                autoComplete="username"
                spellCheck={false}
                aria-invalid={!loginOk}
                required
              />
            </div>
            {!loginOk && login && <p className="pj-bad">Только латиница, цифры, точка, дефис и подчёркивание.</p>}
            {loginChanged && loginOk && (
              <div className="st-note" role="note">
                <span>
                  Входить нужно будет как <b>@{login}</b>. Сессии, токены и участие в проектах сохранятся, задачи с ответственным @{user.login} перейдут на новый логин.
                </span>
              </div>
            )}
          </div>
        </div>
        <div className="st-row">
          <label className="lbl" htmlFor="pf-mail">
            <b>Почта</b>
            <span>Сюда приходят уведомления, если не привязан Telegram.</span>
          </label>
          <div className="ctl">
            <input id="pf-mail" type="email" className="st-input" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} autoComplete="email" />
          </div>
        </div>
        <div className="st-foot">
          <span className="grow">{dirty ? "Есть несохранённые изменения" : ""}</span>
          <button type="button" className="btn ghost" disabled={!dirty} onClick={() => setForm(initial)}>
            Отменить
          </button>
          <button className="btn primary" disabled={!dirty || !loginOk || patch.isPending}>
            Сохранить
          </button>
        </div>
      </form>

      <section className="st-card">
        <div className="st-row">
          <div className="lbl">
            <b>Учётная запись</b>
          </div>
          <dl className="st-facts">
            <dt>На сервере</dt>
            <dd>{user.isAdmin ? "администратор" : "пользователь"}</dd>
            <dt>Проекты</dt>
            <dd>{projects.length ? projects.map((p) => `${p.name} (${PROJECT_ROLE_NAME[p.role]})`).join(", ") : "нет"}</dd>
            {user.created && (
              <>
                <dt>С нами с</dt>
                <dd>{new Date(user.created).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" })}</dd>
              </>
            )}
          </dl>
        </div>
      </section>
    </div>
  );
}

/** Shrink a picture to a centred square: small enough to store and quick to show everywhere. */
async function squareImage(file: File, side = 256): Promise<Blob> {
  const bitmap = await createImageBitmap(file);
  const cut = Math.min(bitmap.width, bitmap.height);
  const canvas = document.createElement("canvas");
  canvas.width = side;
  canvas.height = side;
  const g = canvas.getContext("2d");
  if (!g) throw new Error("Браузер не умеет обрабатывать картинки");
  g.fillStyle = "#16171b";
  g.fillRect(0, 0, side, side);
  g.drawImage(bitmap, (bitmap.width - cut) / 2, (bitmap.height - cut) / 2, cut, cut, 0, 0, side, side);
  bitmap.close();
  return new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("Не удалось сжать картинку"))), "image/jpeg", 0.9));
}

function Photo({ user }: { user: SessionUser }) {
  const set = useSetAvatar();
  const act = useAct();
  const input = useRef<HTMLInputElement>(null);
  return (
    <div className="st-photo">
      <PersonAvatar login={user.login} name={user.name} src={user.avatar} size="xl" prefix="Ваше фото" />
      <div className="acts">
        <div>
          <button type="button" className="btn" disabled={set.isPending} onClick={() => input.current?.click()}>
            <Icon.upload size={13} />
            {user.avatar ? "Заменить" : "Загрузить фото"}
          </button>
          {user.avatar && (
            <button type="button" className="btn ghost" disabled={set.isPending} onClick={() => void act(() => set.mutateAsync({ id: user.id, image: null }), "Фото убрано")}>
              Убрать
            </button>
          )}
        </div>
        <span className="hint">PNG, JPG или WebP до 10 МБ. Обрежем по центру до квадрата.</span>
        <input
          ref={input}
          type="file"
          accept="image/png,image/jpeg,image/webp"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            e.target.value = "";
            if (!file) return;
            void act(async () => {
              if (file.size > 10 * 1024 * 1024) throw new Error("Картинка больше 10 МБ");
              await set.mutateAsync({ id: user.id, image: await squareImage(file) });
            }, "Фото обновлено");
          }}
        />
      </div>
    </div>
  );
}

function SecurityTab() {
  const act = useAct();
  const [pw, setPw] = useState({ current: "", password: "" });
  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Вход и пароль</h2>
        <p>После смены пароля все сессии закрываются, в том числе эта: войдите снова с новым паролем. Токены CLI и MCP продолжают работать.</p>
      </div>
      <form
        className="st-card"
        onSubmit={(e) => {
          e.preventDefault();
          void act(() => request("POST", "/api/auth/password", pw), "Пароль изменён, войдите снова").then((ok) => ok && window.location.assign("/"));
        }}
      >
        <div className="st-row">
          <label className="lbl" htmlFor="pf-cur">
            <b>Текущий пароль</b>
          </label>
          <div className="ctl">
            <input id="pf-cur" type="password" className="st-input" autoComplete="current-password" value={pw.current} onChange={(e) => setPw({ ...pw, current: e.target.value })} required />
          </div>
        </div>
        <div className="st-row">
          <label className="lbl" htmlFor="pf-new">
            <b>Новый пароль</b>
            <span>Не короче 8 символов.</span>
          </label>
          <div className="ctl">
            <input
              id="pf-new"
              type="password"
              className="st-input"
              autoComplete="new-password"
              minLength={8}
              value={pw.password}
              onChange={(e) => setPw({ ...pw, password: e.target.value })}
              required
            />
          </div>
        </div>
        <div className="st-foot">
          <span className="grow" />
          <button className="btn primary" disabled={!pw.current || pw.password.length < 8}>
            Сменить пароль
          </button>
        </div>
      </form>
    </div>
  );
}

function TelegramTab() {
  const channels = useChannels().data;
  const act = useAct();
  const [code, setCode] = useState<string>();
  const linked = channels?.links.find((l) => l.channel === "telegram");
  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Telegram</h2>
        <p>Уведомления и вопросы от агентов приходят в Telegram, отвечать можно прямо там.</p>
      </div>
      <section className="st-card">
        <div className="st-card-body">
          {!channels ? (
            <p className="muted">Загрузка…</p>
          ) : !channels.telegram ? (
            <p className="muted">
              Telegram не настроен на сервере: администратору нужно задать <code>telegram.token</code> в config.json.
            </p>
          ) : linked ? (
            <div className="pj-row">
              <span className="grow">Чат привязан.</span>
              <button type="button" className="btn" onClick={() => void act(() => request("DELETE", "/api/me/channels/telegram"), "Telegram отвязан")}>
                Отвязать
              </button>
            </div>
          ) : (
            <>
              <div>
                <button
                  type="button"
                  className="btn primary"
                  onClick={() =>
                    void act(async () => {
                      const r = await request<{ code: string; instructions: string }>("POST", "/api/me/channels/telegram/code");
                      setCode(r.instructions);
                    })
                  }
                >
                  Привязать Telegram
                </button>
              </div>
              {code && <p className="secret">{code}</p>}
            </>
          )}
        </div>
      </section>
    </div>
  );
}

function KeysTab() {
  const { data } = useLlmKeys();
  const act = useAct();
  const [values, setValues] = useState<Record<string, string>>({});
  const [adding, setAdding] = useState("");
  const providers = data?.providers ?? [];
  const keys = data?.keys ?? {};
  const shared = data?.global ?? {};
  const strict = providers.find((p) => p.strict);
  const rest = providers.filter((p) => !p.strict);
  const shown = rest.filter((p) => keys[p.id] || shared[p.id] || p.id === adding);
  const unset = rest.filter((p) => !keys[p.id] && !shared[p.id]);

  const save = (p: LlmProvider, key: string) =>
    act(async () => {
      await request("PUT", `/api/me/llm-keys/${p.id}`, { key });
      setValues((v) => ({ ...v, [p.id]: "" }));
      setAdding("");
    }, `Ключ ${p.title} сохранён`);

  const card = (p: LlmProvider) => {
    const key = keys[p.id];
    const server = shared[p.id];
    const value = values[p.id] ?? "";
    return (
      <form
        className="st-card"
        key={p.id}
        onSubmit={(e) => {
          e.preventDefault();
          void save(p, value);
        }}
      >
        <div className="st-row">
          <div className="lbl">
            <b>{p.title}</b>
            <span>
              {p.strict
                ? "Прокси этого сервера. Агенты на моделях litellm/… без него не запускаются; ждущие агенты стартуют сразу после сохранения."
                : `Провайдер pi: агент на модели этого провайдера (${p.id}/…) получает ключ в переменной ${p.keyVar} — поверх серверного.${
                    server ? " Без вашего ключа он работает на ключе сервера." : " Без вашего ключа он работает на серверном."
                  }`}
            </span>
          </div>
        </div>
        {key ? (
          <div className="st-row">
            <div className="lbl">
              <b>Сейчас</b>
              <span>Ключ хранится на сервере в зашифрованном виде и больше не показывается.</span>
            </div>
            <div className="ctl" style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap" }}>
              {key.unreadable ? (
                <span className="pj-bad">Ключ сохранён, но сервер не может его прочитать (сменился ключ шифрования). Укажите его заново.</span>
              ) : (
                <span>
                  Ваш ключ задан{key.hint ? ` (${key.hint})` : ""}, обновлён {timeAgo(key.updated)}.
                </span>
              )}
              <button
                type="button"
                className="btn ghost"
                onClick={() => void act(() => request("DELETE", `/api/me/llm-keys/${p.id}`), `Ключ ${p.title} удалён`)}
              >
                Удалить
              </button>
            </div>
          </div>
        ) : server ? (
          <div className="st-row">
            <div className="lbl">
              <b>Сейчас</b>
              <span>
                Администратор поделился ключом со всеми: агенты на моделях {p.id}/… работают на нём
                {p.strict ? "" : ", если у вас нет своего"}. Ваш ключ, если появится, перекроет его.
              </span>
            </div>
            <div className="ctl" style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap" }}>
              {server.unreadable ? (
                <span className="pj-bad">Ключ сервера сохранён, но сервер не может его прочитать (сменился ключ шифрования).</span>
              ) : (
                <span>
                  Ключ сервера задан{server.hint ? ` (${server.hint})` : ""}, обновлён {timeAgo(server.updated)}.
                </span>
              )}
            </div>
          </div>
        ) : null}
        <div className="st-row">
          <label className="lbl" htmlFor={`pf-key-${p.id}`}>
            <b>{key ? "Новый ключ" : server ? "Свой ключ (необязательный)" : "Ключ"}</b>
            <span>Агенты на моделях {p.id}/…, которые работают от вашего имени, ходят к провайдеру с ним.</span>
          </label>
          <div className="ctl">
            <input
              id={`pf-key-${p.id}`}
              type="password"
              className="st-input"
              autoComplete="off"
              placeholder="sk-…"
              value={value}
              onChange={(e) => setValues((v) => ({ ...v, [p.id]: e.target.value }))}
              required
            />
          </div>
        </div>
        <div className="st-foot">
          {!key && p.id === adding && (
            <button type="button" className="btn ghost" onClick={() => setAdding("")}>
              Отмена
            </button>
          )}
          <span className="grow" />
          <button className="btn primary" disabled={!value.trim()}>
            {key ? `Заменить ключ ${p.title}` : server ? `Свой ключ вместо ключа сервера` : `Сохранить ключ ${p.title}`}
          </button>
        </div>
      </form>
    );
  };

  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Провайдеры и ключи</h2>
        <p>
          Агенты, которые работают от вашего имени (команды и задачи, запущенные вами или по вашим задачам, и оркестратор, когда отвечает вам), ходят к моделям с этими
          ключами. Ключ LiteLLM обязателен для моделей <code>litellm/…</code>. Ключи остальных провайдеров, которых поддерживает pi, необязательны: они действуют для
          моделей своего провайдера (<code>anthropic/…</code> — ключ Anthropic) и перекрывают серверный. Ключи, которыми администратор поделился со всеми, стоят рядом и
          действуют без ваших собственных; ваш ключ того же провайдера перекрывает серверный.
        </p>
      </div>
      {strict && card(strict)}
      {shown.map(card)}
      {unset.length > 0 && (
        <div className="st-card">
          <div className="st-row">
            <label className="lbl" htmlFor="pf-add-provider">
              <b>Другой провайдер</b>
              <span>Anthropic, OpenAI, Google, xAI и другие провайдеры pi: выберите, чтобы указать свой ключ.</span>
            </label>
            <div className="ctl">
              <select
                id="pf-add-provider"
                className="st-input"
                value={adding}
                onChange={(e) => setAdding(e.target.value)}
              >
                <option value="">— выбрать провайдера —</option>
                {unset.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.title}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
