// The «Провайдеры» tab of the server settings, for its administrators: the
// provider keys the server shares with everyone — the LiteLLM proxy key an
// agent owes, and the keys of the providers pi supports. A person's own key of
// the same provider comes first; where they keep none, their agents run on the
// shared one.

import { useState } from "react";
import { useSharedLlmKeys, type LlmProvider } from "@/entities/platform";
import { request } from "@/shared/api";
import { timeAgo } from "@/shared/lib";
import { Icon, useRefreshingAct as useAct } from "@/shared/ui";
import { ProviderConnectDialog } from "./ProviderConnectDialog.tsx";
import { HubActions } from "@/widgets/settings-shell";
import "@/shared/ui/settings.css";

export function ProvidersAdminSection() {
  const { data } = useSharedLlmKeys();
  const act = useAct();
  const [values, setValues] = useState<Record<string, string>>({});
  const [connecting, setConnecting] = useState(false);
  const providers = (data?.providers ?? []).filter((p) => p.apiKey);
  const keys = data?.keys ?? {};
  const shown = providers.filter((p) => keys[p.id]);
  const save = (p: LlmProvider, key: string) =>
    act(async () => {
      await request("PUT", `/api/llm-keys/${p.id}`, { key });
      setValues((v) => ({ ...v, [p.id]: "" }));
    }, `Ключ ${p.title} сохранён`);

  const card = (p: LlmProvider) => {
    const key = keys[p.id];
    const value = values[p.id] ?? "";
    return (
      <form
        className="st-card provider-card"
        key={p.id}
        onSubmit={(e) => {
          e.preventDefault();
          void save(p, value);
        }}
      >
        <div className="st-row">
          <div className="lbl"><b>{p.title}</b></div>
        </div>
        {key && (
          <div className="st-row">
            <div className="lbl">
              <b>Сейчас</b>
              <span>Хранится на сервере в зашифрованном виде.</span>
            </div>
            <div className="ctl" style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap" }}>
              {key.unreadable ? (
                <span className="pj-bad">Ключ сохранён, но сервер не может его прочитать (сменился ключ шифрования). Укажите его заново.</span>
              ) : (
                <span>
                  Ключ задан{key.hint ? ` (${key.hint})` : ""}, обновлён {timeAgo(key.updated)}.
                </span>
              )}
              <button
                type="button"
                className="g-btn ghost"
                onClick={() => void act(() => request("DELETE", `/api/llm-keys/${p.id}`), `Ключ ${p.title} удалён`)}
              >
                Удалить
              </button>
            </div>
          </div>
        )}
        <div className="st-row">
          <label className="lbl" htmlFor={`sv-key-${p.id}`}>
            <b>{key ? "Новый ключ" : "Ключ"}</b>
            <span>Общий API-ключ провайдера.</span>
          </label>
          <div className="ctl">
            <input
              id={`sv-key-${p.id}`}
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
          <span className="grow" />
          <button className="g-btn secondary" disabled={!value.trim()}>
            {key ? `Заменить ключ ${p.title}` : `Сохранить ключ ${p.title}`}
          </button>
        </div>
      </form>
    );
  };

  return (
    <div className="st-body">
      {providers.length > 0 && (
        <HubActions>
          <button type="button" className="g-btn secondary" onClick={() => setConnecting(true)}><Icon.plus size={14} />Добавить провайдера</button>
        </HubActions>
      )}
      {shown.map(card)}
      {connecting && <ProviderConnectDialog providers={providers} scope="server" onClose={() => setConnecting(false)} />}
    </div>
  );
}
