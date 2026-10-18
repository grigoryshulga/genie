// Pieces the observation pages share: a timestamp, and the links to tasks.

import { Link } from "react-router";
import { clock, dayLabel } from "@/shared/lib";

/** A timestamp as «Сегодня, 14:05» or «18 октября, 14:05»; an em dash when there is none. */
export function when(iso?: string): string {
  return iso ? `${dayLabel(iso)}, ${clock(iso)}` : "—";
}

/** The task IDs of a case or an incident, each a link to its task page; an em dash for none. */
export function TaskLinks({ ids }: { ids: string[] }) {
  if (!ids.length) return <>—</>;
  return (
    <span className="obs-tasks">
      {ids.map((id) => (
        <Link key={id} className="mono" to={`/task/${encodeURIComponent(id)}`}>
          {id}
        </Link>
      ))}
    </span>
  );
}
