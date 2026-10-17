// The role's skills: all installed ones or a chosen subset, saved as the
// `skills` field of the role file.

import { useState } from "react";
import { type Catalogue, type RoleDetail } from "@/entities/agent-config";

export function useSkillsSettings(detail: RoleDetail) {
  const [only, setOnly] = useState(detail.role.skills !== undefined);
  const [chosen, setChosen] = useState<string[]>(detail.role.skills ?? []);
  return { only, setOnly, chosen, setChosen, value: only ? chosen : undefined };
}

export function SkillsFields({ cfg, state }: { cfg: Catalogue; state: ReturnType<typeof useSkillsSettings> }) {
  const { only, setOnly, chosen, setChosen } = state;
  const [search, setSearch] = useState("");
  const names = [...new Set([...cfg.skills.map((s) => s.name), ...chosen])];
  const visible = names.filter((name) => `${name} ${cfg.skills.find((s) => s.name === name)?.description ?? ""}`.toLocaleLowerCase().includes(search.toLocaleLowerCase()));
  return (
      <div className="ag-choice">
        <label><input type="radio" name="skills-mode" checked={!only} onChange={() => setOnly(false)} />Все установленные навыки</label>
        <label><input type="radio" name="skills-mode" checked={only} onChange={() => setOnly(true)} />Выбрать навыки</label>
        <p className="muted ag-note">Навыки репозитория доступны в любом режиме.</p>
        {only && <>
          <input className="in" type="search" aria-label="Поиск навыков" placeholder="Найти навык…" value={search} onChange={(e) => setSearch(e.target.value)} />
          <div className="ag-checks ag-option-list">
            {visible.map((name) => {
              const skill = cfg.skills.find((s) => s.name === name);
              return <label key={name}>
                <input type="checkbox" checked={chosen.includes(name)} onChange={(e) => setChosen((c) => e.target.checked ? [...c, name] : c.filter((x) => x !== name))} />
                <span><strong>{name}</strong><span className="muted">{skill?.description ?? "Не установлен — агент не получит этот навык"}</span></span>
              </label>;
            })}
            {!visible.length && <p className="muted ag-note">{names.length ? "Ничего не найдено." : "В библиотеке пока нет навыков."}</p>}
          </div>
        </>}
      </div>
  );
}
