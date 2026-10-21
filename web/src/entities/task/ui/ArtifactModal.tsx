import { useState } from "react";
import { bytes } from "@/shared/lib";
import { Icon, ImagePreview, Markdown, Modal } from "@/shared/ui";
import { fetchArtifact } from "../api.ts";

export interface OpenArtifact {
  task: string;
  n: number;
  name: string;
  kind: string;
  size: number;
  text?: string;
  /** Set by the server when the artifact's own bytes are a supported raster image. */
  mime?: string;
}

/** Loads an artifact on demand and shows it in a modal. */
export function useArtifactViewer(onError: (e: Error) => void) {
  const [open, setOpen] = useState<OpenArtifact | undefined>();
  const show = (task: string, n: number) => fetchArtifact(task, n).then((r) => setOpen({ ...r, task, n }), onError);
  const modal = open ? <ArtifactModal artifact={open} onClose={() => setOpen(undefined)} /> : null;
  return { show, modal };
}

export function ArtifactModal({ artifact, onClose }: { artifact: OpenArtifact; onClose: () => void }) {
  return (
    <Modal label={artifact.name} wide onClose={onClose}>
      <div className="mh">
        <Icon.file />
        <span className="g-mono">{artifact.name}</span>
        <span>
          {artifact.kind} · {bytes(artifact.size)}
        </span>
        <span className="grow" />
        <a href={`/api/tasks/${encodeURIComponent(artifact.task)}/artifacts/${artifact.n}?download=1`}>Скачать</a>
        <button type="button" className="g-icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <div className="mb">
        {artifact.mime ? (
          <ImagePreview
            src={`/api/tasks/${encodeURIComponent(artifact.task)}/artifacts/${artifact.n}?raw=1`}
            variant="full"
            alt={artifact.name}
          />
        ) : artifact.text === undefined ? (
          <span className="g-caption">Двоичный файл — скачайте его.</span>
        ) : /\.(md|markdown)$/i.test(artifact.name) ? (
          <Markdown text={artifact.text} />
        ) : (
          <pre className="view">{artifact.text}</pre>
        )}
      </div>
    </Modal>
  );
}
