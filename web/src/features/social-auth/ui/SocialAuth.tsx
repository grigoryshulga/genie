import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { request, type SocialProvider } from "@/shared/api";
import "./social-auth.css";

export function SocialAuth({ link = false }: { link?: boolean }) {
  const { data: providers } = useQuery({
    queryKey: ["social-providers"],
    queryFn: () => request<SocialProvider[]>("GET", "/api/auth/social"),
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const failed = new URLSearchParams(window.location.search).has("socialError");
  if (!providers?.length && !failed && !error) return null;
  return (
    <div className="social-auth" aria-label={link ? "Подключённые способы входа" : "Вход через провайдера"}>
      {failed && <p role="alert" className="auth-error">Не удалось войти через провайдера. Попробуйте ещё раз. Если регистрация закрыта, войдите по паролю и подключите провайдера в настройках профиля.</p>}
      {providers?.map((provider) => (
        <button
          key={provider.id}
          type="button"
          className="btn"
          disabled={busy || (link && provider.connected)}
          onClick={() => {
            setBusy(true);
            setError(undefined);
            void request<{ url: string }>("POST", `/api/auth/social/${provider.id}/start`, { link })
              .then(({ url }) => window.location.assign(url))
              .catch(() => {
                setError("Не удалось начать вход. Проверьте подключение и попробуйте ещё раз.");
                setBusy(false);
              });
          }}
        >
          {link ? (provider.connected ? `${provider.title} подключён` : `Подключить ${provider.title}`) : `Войти через ${provider.title}`}
        </button>
      ))}
      {!link && providers?.some((provider) => provider.registration) && <p className="hint">Первый вход создаст аккаунт. Доступ к проектам выдаёт администратор.</p>}
      {link && !!providers?.length && <p className="hint">Подключённый провайдер позволяет входить в эту учётную запись. Совпадение email не связывает аккаунты.</p>}
      {error && <p role="alert" className="auth-error">{error}</p>}
    </div>
  );
}
