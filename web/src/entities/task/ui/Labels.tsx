import { Tag } from "@/shared/ui";

export function Labels({ labels }: { labels: string[] }) {
  if (!labels.length) return null;
  return (
    <span className="labels">
      {labels.map((l) => (
        <Tag key={l}>{l}</Tag>
      ))}
    </span>
  );
}
