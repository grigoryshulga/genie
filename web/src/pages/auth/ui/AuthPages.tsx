import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent, type ReactNode } from "react";
import { useSearchParams } from "react-router";
import { SocialAuth } from "@/features/social-auth";
import { sessionKey } from "@/entities/session";
import { request } from "@/shared/api";
import { Button, Field, Input } from "@/shared/ui";
import { AuthLayout } from "./AuthLayout";

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

/** The form's own failure (wrong password, a taken login): the server's message, in the danger tone. */
function FormError({ children }: { children: ReactNode }) {
  return (
    <p className="g-error" role="alert">
      {children}
    </p>
  );
}

export function LoginPage({ note }: { note?: string }) {
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const { onSubmit, error, busy } = useSubmit(() => request("POST", "/api/auth/login", { login, password }));
  return (
    <AuthLayout
      slogan="Команда уже в потоке"
      pitch="Задачи, документация и агенты — в одном месте."
      title="Вход"
      note={note}
      foot="Нет аккаунта? Попросите у администратора ссылку-приглашение."
    >
      <form className="auth-form" onSubmit={onSubmit}>
        <Field label="Логин">
          <Input autoFocus autoComplete="username" value={login} onChange={(e) => setLogin(e.target.value)} required />
        </Field>
        <Field label="Пароль">
          <Input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
        </Field>
        {error && <FormError>{error === "wrong login or password" ? "Неверный логин или пароль" : error}</FormError>}
        <Button type="submit" variant="primary" busy={busy}>
          Войти
        </Button>
      </form>
      <p className="auth-or">или</p>
      <SocialAuth />
    </AuthLayout>
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
    <AuthLayout
      slogan="Вас ждут в команде"
      pitch="Ссылка создаёт аккаунт: логин, имя и пароль."
      title="Приглашение"
      note={token ? undefined : "В ссылке нет приглашения — попросите у администратора новую."}
      foot="Приглашение одноразовое: после него вы входите обычным способом."
    >
      <form className="auth-form" onSubmit={onSubmit}>
        <Field label="Логин (латиница)">
          <Input autoFocus autoComplete="username" value={login} onChange={(e) => setLogin(e.target.value)} required />
        </Field>
        <Field label="Как вас называть">
          <Input value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Пароль (не короче 8 символов)">
          <Input type="password" autoComplete="new-password" minLength={8} value={password} onChange={(e) => setPassword(e.target.value)} required />
        </Field>
        {error && <FormError>{error}</FormError>}
        <Button type="submit" variant="primary" busy={busy} disabled={!token}>
          Присоединиться
        </Button>
      </form>
    </AuthLayout>
  );
}
