// «Личное» — the person's own settings, one section each: the account (photo,
// name, login, mail), signing in (password), Telegram, the model keys agents
// use on one's behalf (the LiteLLM proxy and the providers pi supports), and
// personal tokens.

import { useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { SocialAuth } from "@/features/social-auth";
import { PersonAvatar, PROJECT_ROLE_NAME, usePatchUser, useSetAvatar } from "@/entities/project";
import { useLlmKeys, type LlmProvider } from "@/entities/platform";
import { type SessionUser, useSession } from "@/entities/session";
import { request } from "@/shared/api";
import { timeAgo } from "@/shared/lib";
import { Icon, useRefreshingAct as useAct } from "@/shared/ui";
import { PersonalTelegram } from "./TelegramSettings.tsx";
import { TokensTab } from "./TokensTab.tsx";
import { ProviderConnectDialog } from "./ProviderConnectDialog.tsx";
import { AppearanceSettings } from "./AppearanceSettings.tsx";
import "@/shared/ui/settings.css";

export function MeSettingsPage() {
  const session = useSession().data;
  const { section = "account" } = useParams();
  const local = session?.mode === "local";
  return (
    <main className="main">
      <div className="scroll">
        {!session ? null : section === "appearance" ? (
          <AppearanceSettings />
        ) : local ? (
          <div className="st-body">
            <div className="st-head">
              <h2>Локальный режим</h2>
              <p>
                На сервере пока нет пользователей, и он открыт без входа только с этой машины. Создайте <Link to="/manage/server/accounts">учётную запись администратора</Link> в глобальных настройках.
              </p>
            </div>
          </div>
        ) : section === "security" ? (
          <SecurityTab />
        ) : section === "telegram" ? (
          <PersonalTelegram />
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
          <button type="button" className="g-btn ghost" disabled={!dirty} onClick={() => setForm(initial)}>
            Отменить
          </button>
          <button className="g-btn secondary" disabled={!dirty || !loginOk || patch.isPending}>
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

async function cropImage(file: Blob, zoom: number, x: number, y: number, side = 256): Promise<Blob> {
  const bitmap = await createImageBitmap(file);
  const cut = Math.min(bitmap.width, bitmap.height) / zoom;
  const left = (bitmap.width - cut) / 2 + x * (bitmap.width - cut) / 2;
  const top = (bitmap.height - cut) / 2 + y * (bitmap.height - cut) / 2;
  const canvas = document.createElement("canvas");
  canvas.width = side;
  canvas.height = side;
  const g = canvas.getContext("2d");
  if (!g) throw new Error("Браузер не умеет обрабатывать картинки");
  // The JPEG backing of a cropped avatar: a canvas fill, not a UI colour, and it has to
  // be opaque because JPEG has no alpha.
  g.fillStyle = "#16171b";
  g.fillRect(0, 0, side, side);
  g.drawImage(bitmap, left, top, cut, cut, 0, 0, side, side);
  bitmap.close();
  return new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("Не удалось сжать картинку"))), "image/jpeg", 0.9));
}

function Photo({ user }: { user: SessionUser }) {
  const set = useSetAvatar();
  const act = useAct();
  const input = useRef<HTMLInputElement>(null);
  const [source, setSource] = useState<{ blob: Blob; url: string }>();
  const [zoom, setZoom] = useState(1);
  const [x, setX] = useState(0);
  const [y, setY] = useState(0);
  const openCrop = (blob: Blob) => {
    const url = URL.createObjectURL(blob);
    setSource((old) => { if (old) URL.revokeObjectURL(old.url); return { blob, url }; });
    setZoom(1); setX(0); setY(0);
  };
  const closeCrop = () => { if (source) URL.revokeObjectURL(source.url); setSource(undefined); };
  return (
    <div className="st-photo">
      <PersonAvatar login={user.login} name={user.name} src={user.avatar} size="xl" prefix="Ваше фото" />
      <div className="acts">
        <div>
          <button type="button" className="g-btn secondary" disabled={set.isPending} onClick={() => input.current?.click()}>
            <Icon.upload size={13} />
            {user.avatar ? "Заменить" : "Загрузить фото"}
          </button>
          {user.avatar && (
            <>
              <button type="button" className="g-btn ghost" disabled={set.isPending} onClick={() => void act(async () => openCrop(await (await fetch(user.avatar!)).blob()), "")}>Настроить кадр</button>
              <button type="button" className="g-btn ghost" disabled={set.isPending} onClick={() => void act(() => set.mutateAsync({ id: user.id, image: null }), "Фото убрано")}>Убрать</button>
            </>
          )}
        </div>
        <span className="hint">PNG, JPG или WebP до 10 МБ. Настройте масштаб и положение кадра.</span>
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
              openCrop(file);
            }, "");
          }}
        />
      </div>
      {source && <div className="g-scrim st-crop-backdrop" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) closeCrop(); }}>
        <section className="st-crop" role="dialog" aria-modal="true" aria-labelledby="crop-title">
          <h2 id="crop-title">Настройка фото</h2>
          <div className="st-crop-preview"><img src={source.url} alt="Предпросмотр кадра" style={{ transform: `scale(${zoom}) translate(${x * (zoom - 1) * 50 / zoom}%, ${y * (zoom - 1) * 50 / zoom}%)` }} /></div>
          <label>Масштаб <input type="range" min="1" max="3" step="0.01" value={zoom} onChange={(e) => setZoom(Number(e.target.value))} /></label>
          <label>По горизонтали <input type="range" min="-1" max="1" step="0.01" value={x} onChange={(e) => setX(Number(e.target.value))} /></label>
          <label>По вертикали <input type="range" min="-1" max="1" step="0.01" value={y} onChange={(e) => setY(Number(e.target.value))} /></label>
          <footer><button type="button" className="g-btn ghost" onClick={closeCrop}>Отмена</button><button type="button" className="g-btn secondary" disabled={set.isPending} onClick={() => void act(async () => { await set.mutateAsync({ id: user.id, image: await cropImage(source.blob, zoom, x, y) }); closeCrop(); }, "Фото обновлено")}>Сохранить</button></footer>
        </section>
      </div>}
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
      <SocialAuth link />
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
          <button className="g-btn secondary" disabled={!pw.current || pw.password.length < 8}>
            Сменить пароль
          </button>
        </div>
      </form>
    </div>
  );
}

function KeysTab() {
  const { data } = useLlmKeys();
  const act = useAct();
  const [values, setValues] = useState<Record<string, string>>({});
  const [connecting, setConnecting] = useState(false);
  const providers = data?.providers ?? [];
  const keys = data?.keys ?? {};
  const shared = data?.global ?? {};
  const oauth = data?.oauth ?? {};
  const shown = providers.filter((p) => keys[p.id] || shared[p.id] || oauth[p.id]);
  const save = (p: LlmProvider, key: string) =>
    act(async () => {
      await request("PUT", `/api/me/llm-keys/${p.id}`, { key });
      setValues((v) => ({ ...v, [p.id]: "" }));
    }, `Ключ ${p.title} сохранён`);

  const card = (p: LlmProvider) => {
    const key = keys[p.id];
    const server = shared[p.id];
    const login = oauth[p.id];
    const value = values[p.id] ?? "";
    return (
      <div className="st-card provider-card" key={p.id}>
        <div className="st-row">
          <div className="lbl"><b>{p.title}</b></div>
        </div>
        {key ? (
          <div className="st-row">
            <div className="lbl">
              <b>Сейчас</b>
              <span>Ключ хранится на сервере в зашифрованном виде и больше не показывается.</span>
            </div>
            <div className="ctl" style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap" }}>
              {key.unreadable ? (
                <span className="pj-bad">Ключ сервера сохранён, но сервер не может его прочитать (сменился ключ шифрования). Укажите его заново.</span>
              ) : (
                <span>Личный ключ задан{key.hint ? ` (${key.hint})` : ""}, обновлён {timeAgo(key.updated)}.</span>
              )}
              <button
                type="button"
                className="g-btn ghost"
                onClick={() => void act(() => request("DELETE", `/api/me/llm-keys/${p.id}`), `Ключ ${p.title} удалён`)}
              >
                Удалить
              </button>
            </div>
          </div>
        ) : server ? (
          <div className="st-row provider-admin-status">
            {server.unreadable ? (
              <span className="pj-bad provider-admin-unreadable">Ключ сервера сохранён, но сервер не может его прочитать (сменился ключ шифрования).</span>
            ) : (
              <span className="provider-admin-connected"><Icon.check size={15} />Провайдер подключен администратором</span>
            )}
          </div>
        ) : null}
        {login && (
          <div className="st-row">
            <div className="lbl">
              <b>OAuth</b>
              <span>{login.unreadable ? "Учётные данные сохранены, но их нельзя расшифровать. Подключите провайдера заново." : `Вход подключён ${timeAgo(login.updated)}.`}</span>
            </div>
            <div className="ctl" style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap" }}>
              <button type="button" className="g-btn ghost" onClick={() => void act(() => request("DELETE", `/api/me/llm-oauth/provider/${p.id}`), `OAuth ${p.title} отключён`)}>Отключить OAuth</button>
            </div>
          </div>
        )}
        {p.apiKey && (
          <form onSubmit={(e) => { e.preventDefault(); void save(p, value); }}>
            <div className="st-row">
              <label className="lbl" htmlFor={`pf-key-${p.id}`}>
                <b>{key ? "Новый ключ" : server ? "Свой ключ (необязательный)" : "Личный ключ"}</b>
                <span>Для ваших агентов на моделях {p.id}/….</span>
              </label>
              <div className="ctl">
                <input
                  id={`pf-key-${p.id}`}
                  type="password"
                  className="st-input"
                  autoComplete="off"
                  placeholder="Вставьте ключ API"
                  value={value}
                  onChange={(e) => setValues((v) => ({ ...v, [p.id]: e.target.value }))}
                />
              </div>
            </div>
            <div className="st-foot">
              <span className="grow" />
              <button className="g-btn secondary" disabled={!value.trim()}>
                {key ? `Заменить ключ ${p.title}` : server ? "Сохранить личный ключ" : `Сохранить ключ ${p.title}`}
              </button>
            </div>
          </form>
        )}
      </div>
    );
  };

  return (
    <div className="st-body">
      <div className="st-head">
        <div className="provider-headline">
          <h2>Провайдеры и ключи</h2>
          {providers.length > 0 && <button type="button" className="g-btn secondary provider-add" aria-label="Подключить провайдера" title="Подключить провайдера" onClick={() => setConnecting(true)}><Icon.plus size={15} /></button>}
        </div>
        <p>Подключите провайдера через OAuth или сохраните API-ключ. Личные ключи имеют приоритет над ключами сервера.</p>
      </div>
      {shown.map(card)}
      {connecting && <ProviderConnectDialog providers={providers} scope="personal" onClose={() => setConnecting(false)} />}
    </div>
  );
}
