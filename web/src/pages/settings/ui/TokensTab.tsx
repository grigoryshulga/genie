// Personal tokens for the command line, scripts and one's own agent over MCP:
// how to connect, issuing a token (shown once), the list with its uses, revoking.

import { useState } from "react";
import { type UserToken, useIssueToken, useRevokeToken, useTokens } from "@/entities/project";
import { timeAgo, useTick } from "@/shared/lib";
import { ConfirmDialog, Icon, Segmented, useRefreshingAct as useAct, useToast } from "@/shared/ui";
import { SettingsLoading } from "@/widgets/settings-shell";

type Client = "claude" | "codex" | "opencode" | "cursor" | "gemini" | "generic" | "cli";

const CLIENTS: { id: Client; title: string }[] = [
  { id: "claude", title: "Claude Code" },
  { id: "codex", title: "Codex" },
  { id: "opencode", title: "OpenCode" },
  { id: "cursor", title: "Cursor" },
  { id: "gemini", title: "Gemini CLI" },
  { id: "generic", title: "Другой клиент" },
  { id: "cli", title: "CLI" },
];

function howTo(client: Client, origin: string, token: string): { step: string; snippet: string; check: string } {
  switch (client) {
    case "claude":
      return {
        step: "Добавьте Genie как MCP-сервер в Claude Code:",
        snippet: `claude mcp add --transport http genie ${origin}/mcp \\\n  --header "Authorization: Bearer ${token}"`,
        check: "claude mcp list",
      };
    case "codex":
      return {
        step: "Допишите сервер в ~/.codex/config.toml, а токен положите в переменную GENIE_TOKEN:",
        snippet: `[mcp_servers.genie]\nurl = "${origin}/mcp"\nbearer_token_env_var = "GENIE_TOKEN"`,
        check: "codex mcp list",
      };
    case "opencode":
      return {
        step: "Добавьте блок в ~/.config/opencode/opencode.json:",
        snippet: JSON.stringify({ "$schema": "https://opencode.ai/config.json", mcp: { genie: { type: "remote", url: `${origin}/mcp`, headers: { Authorization: `Bearer ${token}` }, enabled: true } } }, null, 2),
        check: "Откройте список MCP-серверов командой /mcp в OpenCode.",
      };
    case "cursor":
      return {
        step: "Добавьте MCP-сервер в .cursor/mcp.json в проекте:",
        snippet: JSON.stringify({ mcpServers: { genie: { url: `${origin}/mcp`, headers: { Authorization: `Bearer ${token}` } } }, }, null, 2),
        check: "Откройте Settings → Cursor Settings → Tools & MCP.",
      };
    case "gemini":
      return {
        step: "Добавьте блок в ~/.gemini/settings.json:",
        snippet: JSON.stringify({ mcpServers: { genie: { httpUrl: `${origin}/mcp`, headers: { Authorization: `Bearer ${token}` } } }, }, null, 2),
        check: "Запустите Gemini CLI и проверьте список MCP-серверов командой /mcp.",
      };
    case "generic":
      return {
        step: "Для MCP-клиента, который принимает заголовки HTTP, укажите URL и Bearer-токен:",
        snippet: `URL: ${origin}/mcp\nAuthorization: Bearer ${token}`,
        check: "Перезапустите клиент и убедитесь, что инструменты Genie появились в списке.",
      };
    case "cli":
      return {
        step: "Задайте переменные окружения для genie и своих скриптов:",
        snippet: `export GENIE_URL=${origin}\nexport GENIE_TOKEN=${token}`,
        check: "genie me show",
      };
  }
}

function day(iso: string): string {
  return new Date(iso).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: new Date(iso).getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
}

export function TokensTab() {
  useTick(60_000);
  const tokens = useTokens(true);
  const issue = useIssueToken();
  const revoke = useRevokeToken();
  const act = useAct();
  const toast = useToast();
  const [label, setLabel] = useState("");
  const [fresh, setFresh] = useState<{ label: string; token: string }>();
  const [mode, setMode] = useState<"tokens" | "connect">("tokens");
  const [client, setClient] = useState<Client>("claude");
  const [revoking, setRevoking] = useState<UserToken>();
  const origin = window.location.origin;
  const how = howTo(client, origin, fresh?.token ?? "<токен>");
  const list = tokens.data ?? [];
  const copy = (text: string) =>
    void navigator.clipboard?.writeText(text).then(
      () => toast("Скопировано"),
      () => toast("Скопируйте вручную", "error"),
    );
  return (
    <div className="st-body tokens-page">
      <div className="st-tabs-row">
        <Segmented label="Токены и подключение" value={mode} onChange={setMode} items={[{ id: "tokens", label: "Токены" }, { id: "connect", label: "Как подключить" }]} />
      </div>
      {mode === "tokens" && <div className="st-col">
        <div className="st-head">
          <h1 className="g-page-title">Токены CLI и MCP</h1>
        </div>

        <form
          className="pj-row"
          onSubmit={(e) => {
            e.preventDefault();
            const name = label.trim() || "без имени";
            void act(async () => {
              const r = await issue.mutateAsync(name);
              setFresh({ label: name, token: r.token });
              setLabel("");
            });
          }}
        >
          <input
            aria-label="Имя токена"
            placeholder="Где будет жить токен: ноутбук, CI, Claude Code…"
            value={label}
            maxLength={80}
            onChange={(e) => setLabel(e.target.value)}
          />
          <button className="g-btn secondary"  disabled={issue.isPending}>
            Выпустить токен
          </button>
        </form>

        {fresh && (
          <div className="st-ok" role="status">
            <span className="line">
              <Icon.check size={14} />
              <span>
                <b>Токен «{fresh.label}» выпущен.</b> Скопируйте его сейчас: больше мы его не покажем.
              </span>
            </span>
            <div className="st-secret">
              <code>{fresh.token}</code>
              <button type="button" className="g-btn secondary" onClick={() => copy(fresh.token)}>
                Скопировать
              </button>
            </div>
          </div>
        )}

        <section className="st-card" aria-label="Выпущенные токены">
          <div className="st-table">
            <div className="tr th">
              <span>Имя</span>
              <span>Выдан</span>
              <span>Последнее использование</span>
              <span />
            </div>
            {list.map((t) => (
              <div key={t.id} className="tr">
                <span className="nm">
                  <span className="ic">
                    <Icon.key size={14} />
                  </span>
                  <b title={t.label}>{t.label || "без имени"}</b>
                </span>
                <span className="when">{day(t.created)}</span>
                <span className={`when${t.lastUsed ? "" : " idle"}`}>{t.lastUsed ? usedAgo(t.lastUsed) : "ещё не использовался"}</span>
                <span className="end">
                  <button type="button" className="g-btn ghost sm revoke" aria-label={`Отозвать токен ${t.label}`} onClick={() => setRevoking(t)}>
                    Отозвать
                  </button>
                </span>
              </div>
            ))}
          </div>
          {!list.length && <div className="st-empty">{tokens.isPending ? <SettingsLoading rows={2} /> : "Токенов нет."}</div>}

        </section>
      </div>}

      {mode === "connect" && <aside className="st-card tokens-guide" aria-label="Как подключить">
        <div className="st-card-body">
          <h3 style={{ margin: 0, fontSize: "var(--g-text-dense)" }}>Как подключить</h3>
          <p className="g-muted tokens-guide-intro">Genie подключается к MCP-клиентам по HTTP: укажите адрес сервера и Bearer-токен. Создайте отдельный токен для каждого устройства или инструмента, чтобы его можно было отозвать отдельно.</p>
          <Segmented label="Клиент" value={client} onChange={setClient} items={CLIENTS.map((c) => ({ id: c.id, label: c.title }))} />
          <ol className="st-steps">
            <li>
              <div>Выпустите токен и назовите его по устройству.</div>
            </li>
            <li>
              <div>
                <span>{how.step}</span>
                <pre>{how.snippet}</pre>
                <div>
                  <button type="button" className="g-btn secondary sm" onClick={() => copy(how.snippet)}>
                    Скопировать
                  </button>
                </div>
              </div>
            </li>
            <li>
              <div>
                <span>Проверьте, что всё работает:</span>
                <pre>{how.check}</pre>
              </div>
            </li>
          </ol>
          <p className="g-muted" style={{ margin: 0, fontSize: "var(--g-text-caption)", lineHeight: 1.5 }}>
            Токен даёт те же права, что у вас в проектах. Не кладите его в репозиторий.
          </p>
        </div>
      </aside>}

      {revoking && (
        <ConfirmDialog
          title="Отозвать токен?"
          confirmLabel="Отозвать"
          danger
          busy={revoke.isPending}
          onClose={() => setRevoking(undefined)}
          onConfirm={() => void act(() => revoke.mutateAsync(revoking.id), `Токен «${revoking.label}» отозван`).then(() => setRevoking(undefined))}
        >
          Всё, что работает с токеном «{revoking.label}», перестанет достучаться до сервера. Вернуть его нельзя, только выпустить новый.
        </ConfirmDialog>
      )}
    </div>
  );
}

function usedAgo(iso: string): string {
  const ago = timeAgo(iso);
  if (ago === "сейчас") return "только что";
  return /^\d+ (мин|ч|д)$/.test(ago) ? `${ago} назад` : ago;
}
