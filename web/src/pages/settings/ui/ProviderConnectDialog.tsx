import { type FormEvent, useEffect, useRef, useState } from "react";
import type { LlmProvider } from "@/entities/platform";
import { request } from "@/shared/api";
import { Icon, Modal, Segmented, useAct, useRefreshingAct } from "@/shared/ui";

type Scope = "personal" | "server";
type Flow = {
  status: "starting" | "waiting" | "complete" | "error" | "cancelled";
  message: string;
  authorization?: { url?: string; code?: string; instructions?: string };
  prompt?: { id: string; method: string; title: string; secret?: boolean; message?: string; placeholder?: string; options?: string[] };
};

export function ProviderConnectDialog({ providers, scope, onClose }: { providers: LlmProvider[]; scope: Scope; onClose: () => void }) {
  const [providerId, setProviderId] = useState("");
  const [mode, setMode] = useState<"key" | "oauth">("key");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [sessionId, setSessionId] = useState("");
  const [flow, setFlow] = useState<Flow | null>(null);
  const [answer, setAnswer] = useState("");
  const [choice, setChoice] = useState("");
  const completed = useRef(false);
  const act = useAct();
  const refreshAct = useRefreshingAct();
  const provider = providers.find((item) => item.id === providerId);
  const allowOAuth = scope === "personal";
  const keyEndpoint = scope === "personal" ? `/api/me/llm-keys/${providerId}` : `/api/llm-keys/${providerId}`;

  useEffect(() => {
    if (!sessionId) return;
    let alive = true;
    let timer = 0;
    const poll = async () => {
      try {
        const state = await request<Flow>("GET", `/api/me/llm-oauth/${sessionId}`);
        if (alive) setFlow(state);
      } catch (e) {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      }
    };
    void poll();
    timer = window.setInterval(() => void poll(), 800);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [sessionId]);

  useEffect(() => {
    if (!sessionId || flow?.status !== "complete" || completed.current) return;
    completed.current = true;
    void refreshAct(() => request("GET", "/api/me/llm-keys"), `${provider?.title ?? "Провайдер"} подключён`);
  }, [flow?.status, provider?.title, refreshAct, sessionId]);

  const close = () => {
    if (sessionId && (flow?.status === "starting" || flow?.status === "waiting" || !flow)) {
      void request("DELETE", `/api/me/llm-oauth/${sessionId}`).catch(() => {});
    }
    onClose();
  };

  const selectProvider = (id: string) => {
    setProviderId(id);
    const selected = providers.find((item) => item.id === id);
    setMode(selected?.apiKey ? "key" : "oauth");
    setKey("");
    setError("");
  };

  const saveKey = async (event: FormEvent) => {
    event.preventDefault();
    if (!provider || !key.trim() || busy) return;
    setBusy(true);
    const done = await refreshAct(() => request("PUT", keyEndpoint, { key }), `Ключ ${provider.title} сохранён`);
    setBusy(false);
    if (done) onClose();
  };

  const startOAuth = async () => {
    if (!provider || busy) return;
    setBusy(true);
    setError("");
    try {
      const result = await request<{ id: string }>("POST", "/api/me/llm-oauth", { provider: provider.id });
      setSessionId(result.id);
      setFlow({ status: "starting", message: "Запускаем OAuth-подключение…" });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const respond = async (body: Record<string, unknown>) => {
    if (!flow?.prompt || !sessionId || busy) return;
    setBusy(true);
    const done = await act(() => request("POST", `/api/me/llm-oauth/${sessionId}/respond`, { id: flow.prompt!.id, ...body }));
    setBusy(false);
    if (done) {
      setFlow((current) => current ? { ...current, prompt: undefined } : current);
      setAnswer("");
      setChoice("");
    }
  };

  const savePrompt = (event: FormEvent) => {
    event.preventDefault();
    void respond({ value: flow?.prompt?.method === "select" ? choice : answer });
  };

  return (
    <Modal label="Подключить провайдера" onClose={close}>
      <div className="mh">
        <span className="grow">Подключение провайдера</span>
        <button type="button" className="g-btn ghost" aria-label="Закрыть" onClick={close}>
          <Icon.close size={13} />
        </button>
      </div>
      <div className="mb provider-connect">
        <label className="lbl" htmlFor="connect-provider">
          <b>Провайдер</b>
        </label>
        <select id="connect-provider" className="st-input" value={providerId} onChange={(e) => selectProvider(e.target.value)} disabled={!!sessionId}>
          <option value="">— выбрать провайдера —</option>
          {providers.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}
        </select>

        {provider && (provider.apiKey && allowOAuth && provider.oauth) && (
          <Segmented label="Способ подключения" value={mode} onChange={setMode} items={[{ id: "oauth", label: "OAuth" }, { id: "key", label: "Ключ API" }]} />
        )}

        {provider && mode === "key" && provider.apiKey && (
          <form className="provider-key-form" onSubmit={(e) => void saveKey(e)}>
            <label className="lbl" htmlFor="provider-key">
              <b>{scope === "personal" ? "Личный API-ключ" : "Общий API-ключ сервера"}</b>
              <span>{provider.keyVar ? `Pi использует переменную ${provider.keyVar}.` : ""}</span>
            </label>
            <input id="provider-key" className="st-input" type="password" autoComplete="new-password" placeholder="Вставьте ключ API" value={key} onChange={(e) => setKey(e.target.value)} required autoFocus />
            <div className="provider-actions">
              <button type="button" className="g-btn ghost" onClick={close}>Отмена</button>
              <button className="g-btn secondary" disabled={!key.trim() || busy}>{busy ? "Сохраняем…" : "Сохранить ключ"}</button>
            </div>
          </form>
        )}

        {provider && mode === "oauth" && allowOAuth && provider.oauth && !sessionId && (
          <div className="provider-oauth-start">
            <p>Войдите в {provider.title} через страницу провайдера. Данные входа сохранятся в вашем профиле.</p>
            <button type="button" className="g-btn secondary" disabled={busy} onClick={() => void startOAuth()}>{busy ? "Запускаем…" : "Продолжить с OAuth"}</button>
          </div>
        )}

        {flow && (
          <div className="provider-oauth-flow" aria-live="polite">
            <p>{flow.message}</p>
            {flow.authorization?.instructions && <p>{flow.authorization.instructions}</p>}
            {flow.authorization?.url && <a className="provider-oauth-link" href={flow.authorization.url} target="_blank" rel="noreferrer">Открыть страницу входа</a>}
            {flow.authorization?.code && <code className="st-code">{flow.authorization.code}</code>}
            {flow.prompt?.message && <p>{flow.prompt.message}</p>}
            {flow.authorization?.url && flow.prompt?.method === "input" && <p className="g-muted">После входа браузер перейдёт на localhost и может показать ошибку — это нормально. Скопируйте адрес из строки браузера целиком и вставьте ниже.</p>}
            {flow.prompt?.method === "confirm" && (
              <div className="provider-actions">
                <button type="button" className="g-btn ghost" disabled={busy} onClick={() => void respond({ confirmed: false })}>Нет</button>
                <button type="button" className="g-btn secondary" disabled={busy} onClick={() => void respond({ confirmed: true })}>Продолжить</button>
              </div>
            )}
            {flow.prompt && flow.prompt.method !== "confirm" && (
              <form className="provider-key-form" onSubmit={savePrompt}>
                <label className="lbl" htmlFor="provider-oauth-answer"><b>{flow.prompt.title}</b></label>
                {flow.prompt.method === "select" ? (
                  <select id="provider-oauth-answer" className="st-input" value={choice} onChange={(e) => setChoice(e.target.value)} required>
                    <option value="">— выбрать —</option>
                    {(flow.prompt.options ?? []).map((option) => <option key={option} value={option}>{option}</option>)}
                  </select>
                ) : flow.prompt.method === "editor" ? (
                  <textarea id="provider-oauth-answer" className="st-input" value={answer} onChange={(e) => setAnswer(e.target.value)} placeholder={flow.prompt.placeholder} required />
                ) : (
                  <input id="provider-oauth-answer" className="st-input" type={flow.prompt.secret ? "password" : "text"} value={answer} onChange={(e) => setAnswer(e.target.value)} placeholder={flow.prompt.placeholder ?? "Введите код подтверждения"} autoComplete={flow.prompt.secret ? "off" : "one-time-code"} required />
                )}
                <div className="provider-actions">
                  <button type="button" className="g-btn ghost" disabled={busy} onClick={() => void respond({ cancelled: true })}>Отмена</button>
                  <button className="g-btn secondary" disabled={busy || (flow.prompt.method === "select" ? !choice : !answer.trim())}>Отправить</button>
                </div>
              </form>
            )}
            {flow.status === "complete" && <button type="button" className="g-btn secondary" onClick={close}>Готово</button>}
            {flow.status === "error" && <button type="button" className="g-btn secondary" onClick={close}>Закрыть</button>}
          </div>
        )}
        {error && <p className="pj-bad" role="alert">{error}</p>}
      </div>
    </Modal>
  );
}
