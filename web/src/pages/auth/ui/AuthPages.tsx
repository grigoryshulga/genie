import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent, type ReactNode } from "react";
import { useSearchParams } from "react-router";
import { SocialAuth } from "@/features/social-auth";
import { sessionKey } from "@/entities/session";
import { request } from "@/shared/api";
import { Icon } from "@/shared/ui";
import { LoginBackdrop } from "./LoginBackdrop";

function AuthCard({
  title,
  sub,
  foot,
  children,
  pageClassName,
  cardClassName,
  backdrop,
}: {
  title: string;
  sub?: string;
  foot?: ReactNode;
  children: ReactNode;
  pageClassName?: string;
  cardClassName?: string;
  backdrop?: ReactNode;
}) {
  return (
    <div className={`auth-page${pageClassName ? ` ${pageClassName}` : ""}`}>
      {backdrop}
      <div className={`auth-card${cardClassName ? ` ${cardClassName}` : ""}`}>
        <div className="auth-head">
          <span className="logo-mark lg">
            <Icon.mark size={34} />
          </span>
          <h1>{title}</h1>
          {sub && <p className="auth-sub">{sub}</p>}
        </div>
        {children}
        {foot && <p className="auth-foot">{foot}</p>}
      </div>
    </div>
  );
}

function useSubmit(fn: () => Promise<unknown>) {
  const qc = useQueryClient();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await fn();
      await qc.resetQueries();
      await qc.invalidateQueries({ queryKey: sessionKey });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return { onSubmit, error, busy };
}

export function LoginPage({ note }: { note?: string }) {
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const { onSubmit, error, busy } = useSubmit(() => request("POST", "/api/auth/login", { login, password }));
  return (
    <AuthCard
      title="Вход в genie"
      sub={note || "Ваша команда уже в потоке"}
      foot="Нет аккаунта? Попросите у администратора ссылку-приглашение."
      pageClassName="login-page"
      cardClassName="login-card"
      backdrop={<LoginBackdrop />}
    >
      <form className="auth-form" onSubmit={onSubmit}>
        <label className="field">
          Логин
          <input autoFocus autoComplete="username" value={login} onChange={(e) => setLogin(e.target.value)} required />
        </label>
        <label className="field">
          Пароль
          <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
        </label>
        {error && <div className="auth-error">{error === "wrong login or password" ? "Неверный логин или пароль" : error}</div>}
        <button className="btn primary" disabled={busy}>
          Войти
        </button>
      </form>
      <SocialAuth />
    </AuthCard>
  );
}

export function InvitePage() {
  const [sp] = useSearchParams();
  const token = sp.get("token") ?? "";
  const [login, setLogin] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const { onSubmit, error, busy } = useSubmit(async () => {
    await request("POST", "/api/auth/invite", { token, login, name, password });
    window.location.assign("/");
  });
  return (
    <AuthCard title="Приглашение в genie">
      <form className="auth-form" onSubmit={onSubmit}>
        <label className="field">
          Логин (латиница)
          <input autoFocus autoComplete="username" value={login} onChange={(e) => setLogin(e.target.value)} required />
        </label>
        <label className="field">
          Как вас называть
          <input value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="field">
          Пароль (не короче 8 символов)
          <input type="password" autoComplete="new-password" minLength={8} value={password} onChange={(e) => setPassword(e.target.value)} required />
        </label>
        {error && <div className="auth-error">{error}</div>}
        <button className="btn primary" disabled={busy || !token}>
          Присоединиться
        </button>
      </form>
    </AuthCard>
  );
}
