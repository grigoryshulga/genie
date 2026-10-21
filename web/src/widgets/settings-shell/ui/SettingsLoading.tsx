import { Skeleton } from "@/shared/ui";

/** A placeholder shaped like a settings card while its data loads. */
export function SettingsLoading({ rows = 3 }: { rows?: number }) {
  return (
    <div className="st-loading" role="status" aria-label="Загрузка">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} height={16} width={i % 2 ? "60%" : "85%"} />
      ))}
    </div>
  );
}
