import { Fragment } from "react";
import { FIXED_PERMISSIONS, PERMISSION_GROUPS } from "@/entities/agent-config";

/** Human labels first; explanations and wire identifiers remain available on demand. */
export function PermissionFields({ wanted, onChange }: { wanted: string[]; onChange?: (value: string[]) => void }) {
  return (
    <div className="ag-perms">
      {PERMISSION_GROUPS.map((g) => (
        <fieldset key={g.title} disabled={!onChange}>
          <legend>{g.title}</legend>
          {g.items.map((p) => (
            <label key={p.id} className={`ag-perm${wanted.includes(p.id) ? " on" : ""}`} title={`${p.text}\nКлюч: ${p.id}`}>
              <input
                type="checkbox"
                checked={wanted.includes(p.id)}
                aria-description={p.text}
                onChange={(e) => onChange?.(e.target.checked ? [...wanted, p.id] : wanted.filter((x) => x !== p.id))}
              />
              <span className="ag-perm-label">{p.title}</span>
            </label>
          ))}
        </fieldset>
      ))}
    </div>
  );
}

export function PermissionReference() {
  return (
    <details className="ag-advanced">
      <summary>Ограничения и технические ключи</summary>
      <p className="muted ag-note">Разрешения проверяет сервер, отзыв действует сразу. {FIXED_PERMISSIONS}</p>
      <dl className="ag-dl">
        {PERMISSION_GROUPS.flatMap((g) => g.items).map((p) => (
          <Fragment key={p.id}>
            <dt>{p.title}</dt>
            <dd><code>{p.id}</code> · {p.text}</dd>
          </Fragment>
        ))}
      </dl>
    </details>
  );
}
