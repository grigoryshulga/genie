import { useRef } from "react";
import { ProjectIcon, type ProjectInfo, useSetProjectIcon } from "@/entities/project";
import { useAct } from "@/shared/ui";

/** Fit a project logo inside a small transparent square without cropping its edges. */
async function iconImage(file: File): Promise<Blob> {
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) throw new Error("Выберите PNG, JPEG или WebP");
  if (file.size > 10 * 1024 * 1024) throw new Error("Выберите изображение до 10 МБ");
  const image = await createImageBitmap(file);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 256;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Не удалось обработать изображение");
    const scale = 256 / Math.max(image.width, image.height);
    const width = image.width * scale;
    const height = image.height * scale;
    context.drawImage(image, (256 - width) / 2, (256 - height) / 2, width, height);
    return await new Promise((resolve, reject) => canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error("Не удалось обработать изображение")), "image/png"));
  } finally {
    image.close();
  }
}

export function ProjectIconSettings({ project, admin }: { project: ProjectInfo; admin: boolean }) {
  const input = useRef<HTMLInputElement>(null);
  const set = useSetProjectIcon();
  const act = useAct();
  return (
    <div className="st-row">
      <div className="lbl">
        <b>Иконка проекта</b>
        <span>В шапке и переключателе проектов. Без изображения показываем монограмму.</span>
      </div>
      <div className="ctl">
        <ProjectIcon name={project.name} src={project.icon} large />
        {admin && <>
          <input ref={input} type="file" aria-label="Загрузить иконку проекта" accept="image/png,image/jpeg,image/webp" hidden onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) void act(async () => set.mutateAsync({ slug: project.slug, image: await iconImage(file) }), "Иконка сохранена");
          }} />
          <div className="td-form-actions">
            <button type="button" className="g-btn secondary sm" disabled={set.isPending} onClick={() => input.current?.click()}>{project.icon ? "Заменить иконку" : "Загрузить иконку"}</button>
            {project.icon && <button type="button" className="g-btn ghost sm" disabled={set.isPending} onClick={() => void act(() => set.mutateAsync({ slug: project.slug, image: null }), "Иконка удалена")}>Удалить иконку</button>}
          </div>
        </>}
      </div>
    </div>
  );
}
