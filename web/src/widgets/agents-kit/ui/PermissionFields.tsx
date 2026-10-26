import { PERMISSION_GROUPS } from "@/entities/agent-config";

import { Icon, IconButton, Tooltip } from "@/shared/ui";

/** Human labels first; explanations and wire identifiers remain available on demand. */
export function PermissionFields({ wanted, onChange }: { wanted: string[]; onChange?: (value: string[]) => void }) {
  return (
    <div className="ag-perms">
      {PERMISSION_GROUPS.map((g) => (
        <fieldset key={g.title}>
          <legend>{g.title}</legend>
          {g.items.map((p) => (
            <div key={p.id} className="role-permission-row"><label className={`ag-perm${wanted.includes(p.id) ? " on" : ""}`} title={`${p.text}\nКлюч: ${p.id}`}>
              <input
                type="checkbox"
                disabled={!onChange}
                checked={wanted.includes(p.id)}
                aria-description={p.text}
                onChange={(e) => onChange?.(e.target.checked ? [...wanted, p.id] : wanted.filter((x) => x !== p.id))}
              />
              <span className="ag-perm-label">{p.title}</span>
            </label>
              <Tooltip text={`${p.text}. Ключ: ${p.id}`}><IconButton label={`О разрешении «${p.title}»`}><Icon.info /></IconButton></Tooltip>
            </div>
          ))}
        </fieldset>
      ))}
    </div>
  );
}
