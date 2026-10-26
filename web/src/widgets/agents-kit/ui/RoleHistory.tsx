import { useState } from "react";
import { roleChangeFields, useConfigHistory, useDeleteConfig, useSaveConfig, type ConfigChange, type RoleDetail } from "@/entities/agent-config";
import { Button, ConfirmDialog, ErrorState, Icon, IconButton, LoadingState, useAct } from "@/shared/ui";

const PAGE_SIZE = 20;
const date = (at: string) => new Intl.DateTimeFormat("ru-RU", { dateStyle: "medium", timeStyle: "medium" }).format(new Date(at));

export function RoleHistory({ detail }: { detail: RoleDetail }) {
  const [cursors, setCursors] = useState<(number | undefined)[]>([undefined]);
  const [open, setOpen] = useState<number>();
  const [restore, setRestore] = useState<ConfigChange>();
  const history = useConfigHistory(`role:${detail.role.id}`, true, cursors.at(-1), PAGE_SIZE + 1);
  const save = useSaveConfig();
  const remove = useDeleteConfig();
  const act = useAct();
  const rows = history.data?.slice(0, PAGE_SIZE) ?? [];
  const move = (next: (number | undefined)[]) => { setCursors(next); setOpen(undefined); };
  return <section className="role-history" aria-label="История роли">
    {history.isPending ? <LoadingState label="Загрузка истории" /> : history.error ? <ErrorState message={history.error.message} onRetry={() => void history.refetch()} /> : !rows.length ? <p className="g-muted">Изменений пока нет.</p> : <div className="role-history-table-wrap">
      <table className="role-history-table">
        <thead><tr><th title={Intl.DateTimeFormat().resolvedOptions().timeZone}>Дата и время</th><th>Автор</th><th>Изменение</th><th>Ревизия</th></tr></thead>
        <tbody>{rows.map((c) => <HistoryRow key={c.id} change={c} open={open === c.id} onToggle={() => setOpen(open === c.id ? undefined : c.id)} onRestore={() => setRestore(c)} />)}</tbody>
      </table>
    </div>}
    <nav className="role-history-pagination" aria-label="Страницы истории">
      <IconButton label="Новее" disabled={cursors.length === 1 || history.isPending} onClick={() => move(cursors.slice(0, -1))}><Icon.back /></IconButton>
      <span>Страница {cursors.length}</span>
      <IconButton label="Старее" disabled={!history.data || history.data.length <= PAGE_SIZE || history.isPending} onClick={() => move([...cursors, rows.at(-1)?.id])}><Icon.chevron /></IconButton>
    </nav>
    {restore && <ConfirmDialog title="Восстановить версию роли?" confirmLabel="Восстановить" onClose={() => setRestore(undefined)} onConfirm={() => {
      const revision = restore;
      setRestore(undefined);
      void act(() => revision.before === undefined ? remove("role", detail.role.id, detail.file.hash) : save("role", detail.role.id, { content: revision.before }, detail.file.hash), "Версия роли восстановлена");
    }}>Текущая роль будет заменена состоянием до правки #{restore.id}. Восстановление добавит новую запись в историю.</ConfirmDialog>}
  </section>;
}

function HistoryRow({ change: c, open, onToggle, onRestore }: { change: ConfigChange; open: boolean; onToggle: () => void; onRestore: () => void }) {
  const changes = roleChangeFields(c);
  return <>
    <tr>
      <td><time dateTime={c.at}>{date(c.at)}</time></td>
      <td>{c.user}</td>
      <td>{c.before === undefined ? "Создание" : c.after === undefined ? "Удаление" : "Изменение"}<span className="role-history-fields">{changes.map((f) => f.key).join(", ")}</span></td>
      <td><div className="role-icon-group"><code>#{c.id}</code><IconButton label={`${open ? "Скрыть сравнение" : "Сравнить"} ревизию ${c.id}`} aria-expanded={open} onClick={onToggle}>{open ? <Icon.close /> : <Icon.columns />}</IconButton></div></td>
    </tr>
    {open && <tr><td colSpan={4} className="role-history-expanded">
      <table className="role-history-diff" aria-label={`Изменения ревизии ${c.id}`}>
        <thead><tr><th>Поле</th><th>До</th><th>После</th></tr></thead>
        <tbody>{changes.map((field) => <tr key={field.key}><th><code>{field.key}</code></th><td><pre>{field.before ?? "Не задано"}</pre></td><td><pre>{field.after ?? "Не задано"}</pre></td></tr>)}</tbody>
      </table>
      <details className="role-history-source"><summary>Полные файлы · {c.path}</summary><div className="role-history-snapshots"><div><h4>До</h4><pre>{c.before ?? "Файла не было"}</pre></div><div><h4>После</h4><pre>{c.after ?? "Файл удалён"}</pre></div></div></details>
      <Button sm onClick={onRestore}>Восстановить до этой правки</Button>
    </td></tr>}
  </>;
}
