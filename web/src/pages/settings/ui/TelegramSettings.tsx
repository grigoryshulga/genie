import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { QRCodeSVG } from "qrcode.react";
import { Link } from "react-router";
import { telegramUrl, useChannels, useProjectTelegram, type TelegramLink } from "@/entities/platform";
import { useProjects } from "@/entities/project";
import { useSession } from "@/entities/session";
import { request } from "@/shared/api";
import { useTick } from "@/shared/lib";
import { ConfirmDialog, useAct, useToast } from "@/shared/ui";
import "./telegram.css";
import { SettingsLoading } from "@/widgets/settings-shell";

export function TelegramSettings({ project, admin = false }: { project: string; admin?: boolean }) {
  useTick(1000);
  const query = useProjectTelegram(project);
  const qc = useQueryClient();
  const act = useAct();
  const toast = useToast();
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [link, setLink] = useState<TelegramLink>();
  const data = query.data;
  const expired = !!link && Date.now() >= Date.parse(link.expiresAt);
  const refresh = () => qc.invalidateQueries({ queryKey: ["project-telegram", project] });
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    await act(action);
    setBusy(false);
  };
  return (
    <div className="st-body">
      <div className="st-head">
        <h1 className="g-page-title">Telegram</h1>
      </div>
      {query.isPending ? (
        <SettingsLoading />
      ) : query.error ? (
        <div role="alert">
          {query.error.message}{" "}
          <button className="g-btn secondary" onClick={() => void query.refetch()}>
            Повторить
          </button>
        </div>
      ) : (
        <>
          <section className="st-card">
            <div className="st-card-head">
              <h3>{data?.username ? `@${data.username}` : "Бот проекта"}</h3>
              <span className="g-muted" role="status">
                {!data?.available ? (data?.username ? "Требует настройки" : "Не настроен") : data.connected ? "Подключено" : "Готов к подключению"}
              </span>
            </div>
            <div className="st-card-body tg-connect">
              {!data?.available ? (
                <p className="g-muted">
                  {data?.username ? "Токен недоступен. Администратору нужно сохранить его заново." : "Администратору проекта нужно подключить бота."}
                </p>
              ) : data.connected ? (
                <div className="pj-row tg-connected">
                  <span className="grow">Уведомления приходят в личный чат.</span>
                  <button
                    type="button"
                    className="g-btn secondary"
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await request("POST", telegramUrl(project, "/test"));
                        toast("Тестовое уведомление отправлено");
                      })
                    }
                  >
                    Проверить доставку
                  </button>
                  <button
                    type="button"
                    className="g-btn secondary"
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await request("DELETE", telegramUrl(project, "/link"));
                        setLink(undefined);
                        await refresh();
                      })
                    }
                  >
                    Отключить уведомления
                  </button>
                </div>
              ) : (
                <>
                  {!link || expired ? (
                    <button
                      type="button"
                      className="g-btn secondary"
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          setLink(await request<TelegramLink>("POST", telegramUrl(project, "/link")));
                        })
                      }
                    >
                      {expired ? "Получить новую ссылку" : "Подключить уведомления"}
                    </button>
                  ) : (
                    <div className="tg-pair">
                      {/* A QR code must stay dark-on-light in both themes to scan, so it keeps the
                          library's black-on-white default and sits on a paper chip (`.tg-qr`). */}
                      <div className="tg-qr">
                        <QRCodeSVG
                          value={link.url}
                          size={180}
                          level="M"
                          marginSize={4}
                          title="Подключить Telegram к genie"
                        />
                      </div>
                      <div className="tg-steps">
                        <h3>Откройте бота и нажмите «Старт»</h3>
                        <p className="g-muted">Или отсканируйте QR-код камерой телефона.</p>
                        <a className="g-btn secondary" href={link.url} target="_blank" rel="noreferrer">
                          Открыть в Telegram ↗
                        </a>
                        <button
                          type="button"
                          className="g-btn secondary"
                          onClick={() =>
                            void act(async () => {
                              await navigator.clipboard.writeText(link.url);
                              toast("Ссылка скопирована");
                            })
                          }
                        >
                          Скопировать ссылку
                        </button>
                        <small className="g-muted">Личная ссылка действует 30 минут. Не передавайте её другим.</small>
                        <span role="status" className="g-muted">
                          Ожидаем подключения…
                        </span>
                        <button
                          type="button"
                          className="g-btn ghost"
                          disabled={busy}
                          onClick={() =>
                            void run(async () => {
                              setLink(await request<TelegramLink>("POST", telegramUrl(project, "/link")));
                            })
                          }
                        >
                          Обновить ссылку
                        </button>
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>
          </section>
          {admin && (
            <form
              className="st-card"
              onSubmit={(e) => {
                e.preventDefault();
                void run(async () => {
                  await request("PUT", telegramUrl(project), { token });
                  setToken("");
                  setLink(undefined);
                  await refresh();
                  toast("Бот сохранён");
                });
              }}
            >
              <div className="st-card-head">
                <h3>{data?.username ? "Управление ботом" : "Подключить бота проекта"}</h3>
              </div>
              <div className="st-card-body tg-connect">
                <p>
                  Создайте бота в{" "}
                  <a href="https://t.me/BotFather" target="_blank" rel="noreferrer">
                    @BotFather
                  </a>{" "}
                  командой <code>/newbot</code> и вставьте токен.
                </p>
                <label htmlFor="tg-token">Токен бота</label>
                <input
                  id="tg-token"
                  className="st-input"
                  type="password"
                  autoComplete="off"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder={data?.username ? "Новый токен" : "123456789:…"}
                  required
                />
                <small className="g-muted">Токен хранится зашифрованным. Нужен отдельный бот без webhook.</small>
                {data?.username && <small className="g-muted">При замене бота пользователям понадобится подключиться заново.</small>}
              </div>
              <div className="st-foot">
                {data?.username && (
                  <button type="button" className="g-btn secondary" disabled={busy} onClick={() => setRemoving(true)}>
                    Удалить бота
                  </button>
                )}
                <span className="grow" />
                <button className="g-btn secondary" disabled={busy || !token.trim()}>
                  {busy ? "Проверяем…" : "Проверить и сохранить"}
                </button>
              </div>
            </form>
          )}
          {!admin && <Link to="/manage/project/channels">Настройки бота проекта →</Link>}
        </>
      )}
      {removing && (
        <ConfirmDialog
          title="Удалить бота проекта?"
          confirmLabel="Удалить"
          danger
          onClose={() => setRemoving(false)}
          busy={busy}
          onConfirm={() =>
            void run(async () => {
              await request("DELETE", telegramUrl(project));
              setLink(undefined);
              await refresh();
              setRemoving(false);
            })
          }
        >
          Все подключения будут сброшены. Подписки на задачи сохранятся.
        </ConfirmDialog>
      )}
    </div>
  );
}

export function PersonalTelegram() {
  const projects = useProjects();
  const session = useSession().data;
  const [chosen, setChosen] = useState("");
  const list = projects.data ?? [];
  const project = list.find((p) => p.slug === (chosen || session?.project)) ?? list[0];
  return (
    <>
      <div className="st-body tg-project">
        <label htmlFor="tg-project">Проект</label>
        <select id="tg-project" className="st-input" value={project?.slug ?? ""} onChange={(e) => setChosen(e.target.value)}>
          {list.map((p) => (
            <option key={p.slug} value={p.slug}>
              {p.name}
            </option>
          ))}
        </select>
        {projects.error && <p role="alert">{projects.error.message}</p>}
      </div>
      {project && <TelegramSettings key={project.slug} project={project.slug} />}
      <LegacyTelegram />
    </>
  );
}

function LegacyTelegram() {
  const query = useChannels();
  const act = useAct();
  const qc = useQueryClient();
  const [link, setLink] = useState<{ code: string; bot: string | null }>();
  const [busy, setBusy] = useState(false);
  const linked = query.data?.links.some((l) => l.channel === "telegram");
  if (!query.data?.telegram) return null;
  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    await act(fn);
    setBusy(false);
  };
  return (
    <div className="st-body">
      <section className="st-card">
        <div className="st-card-head">
          <h3>Общий бот сервера</h3>
          <span className="g-muted">Для проектов без своего бота</span>
        </div>
        <div className="st-card-body tg-connect">
          {linked ? (
            <>
              <p>Чат подключён.</p>
              <button
                className="g-btn secondary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await request("DELETE", "/api/me/channels/telegram");
                    setLink(undefined);
                    await qc.invalidateQueries({ queryKey: ["channels"] });
                  })
                }
              >
                Отключить
              </button>
            </>
          ) : (
            <>
              <button
                className="g-btn secondary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    setLink(await request("POST", "/api/me/channels/telegram/code"));
                  })
                }
              >
                Получить ссылку
              </button>
              {link &&
                (link.bot ? (
                  <>
                    <a className="g-btn secondary" href={`https://t.me/${link.bot}?start=${link.code}`} target="_blank" rel="noreferrer">
                      Открыть в Telegram ↗
                    </a>
                    <div className="tg-qr">
                      <QRCodeSVG value={`https://t.me/${link.bot}?start=${link.code}`} size={180} marginSize={4} title="Подключить общий бот" />
                    </div>
                    <small className="g-muted">Личная ссылка действует 30 минут.</small>
                  </>
                ) : (
                  <p>
                    Отправьте боту <code>/start {link.code}</code>. Код действует 30 минут.
                  </p>
                ))}
            </>
          )}
        </div>
      </section>
    </div>
  );
}
