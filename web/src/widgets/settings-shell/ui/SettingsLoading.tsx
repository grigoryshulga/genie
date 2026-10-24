import { LoadingState } from "@/shared/ui";

/** A placeholder shaped like a settings card while its data loads. */
export function SettingsLoading({ rows = 3 }: { rows?: number }) {
  return <LoadingState label="Загрузка" lines={rows} />;
}
