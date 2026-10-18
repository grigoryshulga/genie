import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { useSearchParams } from "react-router";
import { sessionKey } from "@/entities/session";
import { request } from "@/shared/api";
import { Icon } from "@/shared/ui";

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

type TeamMember = { name: string; role: string; activity: string; kind: "person" | "agent" };

const memberSlots: Omit<TeamMember, "name">[] = [
  { role: "Человек", activity: "в команде", kind: "person" },
  { role: "Агент · код", activity: "собирает решение", kind: "agent" },
  { role: "Агент · анализ", activity: "изучает контекст", kind: "agent" },
  { role: "Человек", activity: "проверяет задачу", kind: "person" },
  { role: "Агент · план", activity: "сверяет шаги", kind: "agent" },
];

function createTeamMembers(previousRoster?: string | null): TeamMember[] {
  const names = ["Алексей", "Мария", "Даниил", "София", "Илья", "Вера", "Никита", "Анна", "Марк", "Лена", "Миша", "Саша"];
  for (let i = names.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [names[i], names[j]] = [names[j], names[i]];
  }
  const members = memberSlots.map((slot, index) => ({ ...slot, name: names[index] }));
  if (members.map((member) => member.name).join("|") === previousRoster) [members[0].name, members[1].name] = [members[1].name, members[0].name];
  return members;
}

function LoginBackdrop({ members }: { members: TeamMember[] }) {
  return (
    <div className="login-universe" aria-hidden="true">
      <div className="login-nebula login-nebula-a" />
      <div className="login-nebula login-nebula-b" />
      <div className="login-stars" />
      <div className="login-star-dust" />
      <div className="login-shooting-star" />
      <svg className="login-network" viewBox="0 0 1200 800" preserveAspectRatio="none">
        <defs>
          <radialGradient id="login-hub-glow">
            <stop stopColor="#d8c7ff" stopOpacity=".95" />
            <stop offset="1" stopColor="#9275fa" stopOpacity="0" />
          </radialGradient>
        </defs>
        <g className="login-network-routes">
          <path id="login-route-1" d="M105 185 C225 185 210 350 380 372 S575 400 684 400" />
          <path id="login-route-2" d="M145 570 C275 570 258 428 385 400 S568 400 684 400" />
          <path id="login-route-3" d="M400 105 C402 205 366 290 385 400 S574 400 684 400" />
          <path id="login-route-4" d="M425 670 C420 580 370 505 385 400 S575 400 684 400" />
          <path id="login-route-5" d="M325 385 C420 310 482 325 558 370 S628 400 684 400" />
          <path id="login-route-6" d="M105 185 C205 325 162 510 145 570" />
          <path id="login-route-7" d="M400 105 C534 180 526 272 425 370 S282 520 145 570" />
          <path id="login-route-8" d="M325 385 C365 520 475 585 590 505 S645 425 684 400" />
        </g>
        <g className="login-network-pulses">
          <path d="M105 185 C225 185 210 350 380 372 S575 400 684 400" />
          <path d="M145 570 C275 570 258 428 385 400 S568 400 684 400" />
          <path d="M400 105 C402 205 366 290 385 400 S574 400 684 400" />
          <path d="M425 670 C420 580 370 505 385 400 S575 400 684 400" />
          <path d="M325 385 C420 310 482 325 558 370 S628 400 684 400" />
          <path d="M105 185 C205 325 162 510 145 570" />
          <path d="M400 105 C534 180 526 272 425 370 S282 520 145 570" />
          <path d="M325 385 C365 520 475 585 590 505 S645 425 684 400" />
        </g>
        <circle className="login-network-halo" cx="684" cy="400" r="43" fill="url(#login-hub-glow)" />
        <circle className="login-network-core" cx="684" cy="400" r="7" />
      </svg>
      <div className="login-network-label">общий поток<span>5 участников · 8 связей</span></div>
      {members.map((member, index) => (
        <div className={`login-member login-member-${index + 1}`} key={`member-${index}`}>
          <span className={`login-member-avatar${member.kind === "agent" ? " is-agent" : ""}`}>{member.kind === "agent" ? "✳" : "◉"}</span>
          <span className="login-member-copy">
            <span className="login-member-name">{member.name}</span>
            <span className="login-member-role">{member.role}</span>
            <span className="login-member-activity"><i />{member.activity}</span>
          </span>
        </div>
      ))}
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
  const [members] = useState(() => {
    let previousRoster: string | null = null;
    try {
      previousRoster = window.sessionStorage.getItem("genie-login-roster");
    } catch {
      // The illustration remains useful when browser storage is disabled.
    }
    return createTeamMembers(previousRoster);
  });
  useEffect(() => {
    const roster = members.map((member) => member.name).join("|");
    try {
      window.sessionStorage.setItem("genie-login-roster", roster);
    } catch {
      // Names still change when browser storage is disabled.
    }
  }, [members]);
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
      backdrop={<LoginBackdrop members={members} />}
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
