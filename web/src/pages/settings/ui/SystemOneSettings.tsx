import { useDecisionSettings } from "@/entities/decision";
import { useSession } from "@/entities/session";
import { Link } from "react-router";

export function SystemOneSettings() {
  const slug = useSession().data?.project ?? "";
  const query = useDecisionSettings(slug);
  const data = query.data;
  return (
    <div className="st-body">
      <section className="st-card">
        <div className="st-card-head">
          <h3>Jev / System One</h3>
          <p>Подключение сервиса решений на сервере. Политика использования задаётся в инструментах каждого проекта.</p>
        </div>
        <div className="st-card-body">
          {data ? (
            <>
              <p>
                {data.available ? "Сервис включён" : "Сервис выключен"} · модель <code>{data.model}</code>
              </p>
              <p>
                API: <code>{data.endpoint ?? "не настроен"}</code>
              </p>
              <p>
                Провайдер ключа: <code>{data.keyProvider}</code>
                {data.serverKey ? " · ключ из окружения сервера" : " · общий ключ провайдера"}
              </p>
              <Link className="btn" to="/manage/server/providers">
                Настроить ключ провайдера
              </Link>
              <p className="muted">Адрес API, модель и включение сервиса задаются в секции decisions файла config.json сервера.</p>
            </>
          ) : (
            <p role="status">{query.error?.message ?? "Загрузка…"}</p>
          )}
        </div>
      </section>
    </div>
  );
}
