/**
 * The arcs motif: concentric quarter rings anchored to the bottom-right corner and cropped by it,
 * a green outer band, an orange band inside it and a solid green core. One composition per surface,
 * never behind text or controls (docs/assets/brand/design-system.md). The colours live in board.css
 * and come from the tokens, so both themes follow.
 */
export function Arcs({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 500 500" fill="none" aria-hidden="true" focusable="false">
      <g transform="translate(500 500)">
        <path d="M-500 0A500 500 0 0 1 0-500V-360A360 360 0 0 0-360 0Z" />
        <path d="M-250 0A250 250 0 0 1 0-250V-140A140 140 0 0 0-140 0Z" />
        <path d="M-105 0A105 105 0 0 1 0-105V0Z" />
      </g>
    </svg>
  );
}
