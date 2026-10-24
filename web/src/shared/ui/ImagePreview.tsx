import { useEffect, useRef, useState } from "react";
import { Dialog } from "./kit/overlays.tsx";
import "./ImagePreview.css";

export interface ImagePreviewProps {
  /** Local API URL of the image (artifact raw bytes or a contained repo file). */
  src: string;
  /** Thumbnail for lists, inline for chat/modal content, full for the artifact modal body. */
  variant?: "thumb" | "inline" | "full";
  alt?: string;
  caption?: string;
  className?: string;
  /** When false the image is display-only (use inside an existing button/row). */
  interactive?: boolean;
  /** Called when the image cannot be loaded, so the caller can fall back to text. */
  onUnavailable?: () => void;
}

/**
 * Inline raster preview with a click-to-open lightbox (zoom, pan, Esc/backdrop
 * close, double-click reset). Renders nothing when the image fails to load and
 * lets the caller decide how to degrade — this module never draws a broken image.
 */
export function ImagePreview({ src, variant = "thumb", alt, caption, className, interactive = true, onUnavailable }: ImagePreviewProps) {
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState(false);

  useEffect(() => setFailed(false), [src]);
  if (failed) return null;

  const image = (
    <img
      src={src}
      alt={alt ?? caption ?? ""}
      loading="lazy"
      draggable={false}
      onError={() => {
        setFailed(true);
        onUnavailable?.();
      }}
    />
  );
  const cls = `imgprev imgprev-${variant}${className ? ` ${className}` : ""}`;
  const label = caption ?? alt ?? "Открыть изображение";

  if (!interactive) return <span className={cls} title={label}>{image}</span>;

  return (
    <>
      <button type="button" className={cls} onClick={() => setOpen(true)} title={label}>
        {image}
      </button>
      {open && <Lightbox src={src} alt={alt ?? caption} onClose={() => setOpen(false)} />}
    </>
  );
}

function Lightbox({ src, alt, onClose }: { src: string; alt?: string; onClose: () => void }) {
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | undefined>(undefined);
  const stage = useRef<HTMLDivElement>(null);

  const zoomBy = (factor: number) => setScale((s) => Math.min(8, Math.max(0.25, s * factor)));
  const reset = () => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  };

  // Lock page scroll for as long as the lightbox is open.
  useEffect(() => {
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, []);

  // Native non-passive listener so wheel zoom does not scroll the page behind.
  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomBy(e.deltaY < 0 ? 1.15 : 1 / 1.15);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  return (
    <Dialog title={alt ?? "Изображение"} onClose={onClose} bare className="lightbox-dialog">
      <div
        className="lightbox"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) onClose();
        }}
      >
        <div
          ref={stage}
          className="lightbox-stage"
          onMouseDown={(e) => {
            e.preventDefault();
            drag.current = { x: e.clientX, y: e.clientY, ox: offset.x, oy: offset.y };
          }}
          onMouseMove={(e) => {
            const d = drag.current;
            if (d) setOffset({ x: d.ox + e.clientX - d.x, y: d.oy + e.clientY - d.y });
          }}
          onMouseUp={() => {
            drag.current = undefined;
          }}
          onMouseLeave={() => {
            drag.current = undefined;
          }}
          onDoubleClick={reset}
        >
          <img src={src} alt={alt ?? ""} draggable={false} style={{ transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})` }} />
        </div>
        <div className="lightbox-tools" onMouseDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
          <button type="button" onClick={() => zoomBy(1 / 1.2)} aria-label="Уменьшить">
            −
          </button>
          <button type="button" onClick={reset} aria-label="Сбросить масштаб">
            {Math.round(scale * 100)}%
          </button>
          <button type="button" onClick={() => zoomBy(1.2)} aria-label="Увеличить">
            +
          </button>
          <button type="button" className="lightbox-close" onClick={onClose} aria-label="Закрыть">
            ✕
          </button>
        </div>
      </div>
    </Dialog>
  );
}
