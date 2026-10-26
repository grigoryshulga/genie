import type { SVGProps } from "react";

export type IconProps = SVGProps<SVGSVGElement> & { size?: number };
export const stroke = (size = 14, p: IconProps) => ({ width: size, height: size, viewBox: "0 0 16 16", fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true, ...p });

export const Icon = {
  fit: ({ size, ...p }: IconProps) => <svg {...stroke(size, p)}><path d="M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4" /><rect x="5" y="5" width="6" height="6" rx="1" /></svg>,
  expand: ({ size, ...p }: IconProps) => <svg {...stroke(size, p)}><path d="M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4M2 2l4 4M14 2l-4 4M14 14l-4-4M2 14l4-4" /></svg>,
  collapse: ({ size, ...p }: IconProps) => <svg {...stroke(size, p)}><path d="M6 2v4H2M10 2v4h4M10 14v-4h4M6 14v-4H2M2 2l4 4M14 2l-4 4M14 14l-4-4M2 14l4-4" /></svg>,
  eye: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M1 8s2.5-4.5 7-4.5S15 8 15 8s-2.5 4.5-7 4.5S1 8 1 8Z" />
      <circle cx="8" cy="8" r="2" />
    </svg>
  ),
  eyeOff: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M1 8s2.5-4.5 7-4.5S15 8 15 8s-2.5 4.5-7 4.5S1 8 1 8ZM2 2l12 12" />
    </svg>
  ),
  restart: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M13 8a5 5 0 1 1-1.6-3.7" />
      <path d="M13 3v3h-3" />
    </svg>
  ),
  plus: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M8 3v10M3 8h10" />
    </svg>
  ),
  close: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M4 4l8 8M12 4l-8 8" />
    </svg>
  ),
  search: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5L14 14" />
    </svg>
  ),
  list: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M2.5 4h11M2.5 8h11M2.5 12h11" />
    </svg>
  ),
  filter: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M2 3h12l-4.6 5.4v4.1l-2.8 1.5V8.4L2 3Z" />
    </svg>
  ),
  board: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <rect x="2" y="2.5" width="3.2" height="11" rx="1" />
      <rect x="6.4" y="2.5" width="3.2" height="7" rx="1" />
      <rect x="10.8" y="2.5" width="3.2" height="9" rx="1" />
    </svg>
  ),
  file: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M4 1.5h5.5L12.5 4.5v10h-8.5z" />
      <path d="M9.5 1.5v3h3" />
    </svg>
  ),
  send: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.7}>
      <path d="M8 13V3M3.5 7.5L8 3l4.5 4.5" />
    </svg>
  ),
  chevron: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M6 3l5 5-5 5" />
    </svg>
  ),
  back: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M10 3L5 8l5 5" />
    </svg>
  ),
  menu: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" />
    </svg>
  ),
  check: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.8}>
      <path d="M3.5 8.5l3 3 6-7" />
    </svg>
  ),
  trash: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M2.5 4h11M6 4V2.5h4V4M4 4l.7 9.5h6.6L12 4M6.8 6.5v5M9.2 6.5v5" />
    </svg>
  ),
  stop: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <rect x="3.5" y="3.5" width="9" height="9" rx="1.5" />
    </svg>
  ),
  folder: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.8l1.5 1.5h4.7A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" />
    </svg>
  ),
  server: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <rect x="2.5" y="2.5" width="11" height="4.5" rx="1.2" />
      <rect x="2.5" y="9" width="11" height="4.5" rx="1.2" />
      <path d="M5 4.75h.01M5 11.25h.01" />
    </svg>
  ),
  chart: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M2.5 13.5h11" />
      <path d="M4 11V7.5M8 11V3.5M12 11V5.5" />
    </svg>
  ),
  user: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <circle cx="8" cy="5.5" r="2.5" />
      <path d="M3.5 13.5c.6-2.3 2.3-3.5 4.5-3.5s3.9 1.2 4.5 3.5" />
    </svg>
  ),
  userPlus: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <circle cx="6.5" cy="5.5" r="2.5" />
      <path d="M2 13.5c.6-2.3 2.3-3.5 4.5-3.5s3.9 1.2 4.5 3.5M12.5 5v4M10.5 7h4" />
    </svg>
  ),
  spark: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M8 1.5v4M8 10.5v4M1.5 8h4M10.5 8h4M3.4 3.4l2.2 2.2M10.4 10.4l2.2 2.2M12.6 3.4l-2.2 2.2M5.6 10.4l-2.2 2.2" />
    </svg>
  ),
  /**
   * genie mark: the logo in the sidebar and on public pages. Line art of the
   * genie in docs/assets/brand/genie-mark.svg, stroked with the surrounding
   * color (the tile behind it sets the periwinkle).
   */
  mark: ({ size = 16, ...p }: IconProps) => (
    <svg width={size} height={size} viewBox="90 30 665 700" fill="none" stroke="currentColor" strokeWidth={44} aria-hidden="true" {...p}>
      <path d="M 290 442 C 178 442 161 337 246 307" />
      <rect x="122" y="442" width="536" height="256" rx="96" />
      <g transform="translate(410 236)">
        <circle r="176" />
        <path d="M 162 -51 L 322 -109" strokeLinecap="butt" />
        <path d="M -33 0 A 64 64 0 0 1 95 0" strokeWidth={34} strokeLinecap="butt" />
      </g>
    </svg>
  ),
  bolt: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M9 1.8L3.5 9h4l-1 5.2L12.5 7h-4z" />
    </svg>
  ),
  bell: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3z" />
      <path d="M6.5 14h3" />
    </svg>
  ),
  proposal: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M4 2.5v6.5a3 3 0 0 0 3 3h5" />
      <path d="M10 9.5l2.5 2.5L10 14.5" />
    </svg>
  ),
  gear: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} viewBox="0 0 24 24" strokeWidth={1.7}>
      <path d="M19.14 12.94c.04-.3.06-.61.06-.94s-.02-.64-.07-.94l2.03-1.58a.5.5 0 0 0 .12-.61l-1.92-3.32a.5.5 0 0 0-.61-.22l-2.39.96a7.28 7.28 0 0 0-1.63-.94l-.36-2.54a.5.5 0 0 0-.49-.42h-3.84a.5.5 0 0 0-.49.42L9.19 5.35c-.59.25-1.13.56-1.63.94l-2.39-.96a.5.5 0 0 0-.61.22L2.64 8.87a.5.5 0 0 0 .12.61l2.03 1.58c-.05.31-.07.62-.07.94s.02.63.07.94l-2.03 1.58a.5.5 0 0 0-.12.61l1.92 3.32c.12.22.38.31.61.22l2.39-.96c.5.38 1.04.7 1.63.94l.36 2.54c.04.24.24.42.49.42h3.84c.25 0 .45-.18.49-.42l.36-2.54c.59-.24 1.13-.56 1.63-.94l2.39.96c.23.09.49 0 .61-.22l1.92-3.32a.5.5 0 0 0-.12-.61l-2.03-1.58Z" />
      <circle cx="12" cy="12" r="3.5" />
    </svg>
  ),
  logout: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M6.5 2.5H3.5v11h3M10 5l3 3-3 3M13 8H6.5" />
    </svg>
  ),
  updown: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M5 6l3-3 3 3M5 10l3 3 3-3" />
    </svg>
  ),
  upload: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.6}>
      <path d="M8 11V3M4.5 6.5L8 3l3.5 3.5M3 13h10" />
    </svg>
  ),
  download: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}><path d="M8 2v8M4.5 6.5L8 10l3.5-3.5M3 13h10" /></svg>
  ),
  key: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <circle cx="5.5" cy="10.5" r="3" />
      <path d="M7.6 8.4L13.5 2.5M11.5 4.5l1.5 1.5M10 6l1.2 1.2" />
    </svg>
  ),
  info: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.4}>
      <circle cx="8" cy="8" r="6.2" />
      <path d="M8 7.2v4" />
      <circle cx="8" cy="5" r="0.8" fill="currentColor" stroke="none" />
    </svg>
  ),
  alert: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <circle cx="8" cy="8" r="6.2" />
      <path d="M8 4.8V8.6" />
      <circle cx="8" cy="11.1" r="0.8" fill="currentColor" stroke="none" />
    </svg>
  ),
  branch: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <circle cx="4.5" cy="3.5" r="1.6" />
      <circle cx="4.5" cy="12.5" r="1.6" />
      <circle cx="11.5" cy="5.5" r="1.6" />
      <path d="M4.5 5.1v5.8M11.5 7.1c0 2.4-2 3.2-7 3.9" />
    </svg>
  ),
  merge: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <circle cx="4" cy="3.5" r="1.8" />
      <circle cx="4" cy="12.5" r="1.8" />
      <circle cx="12" cy="8.5" r="1.8" />
      <path d="M4 5.3v5.4M4 5.3c0 2.6 2.4 3.2 6.2 3.2" />
    </svg>
  ),
  external: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)}>
      <path d="M6 3H3v10h10v-3M9 3h4v4M13 3L7 9" />
    </svg>
  ),
  dots: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} fill="currentColor" stroke="none">
      <circle cx="3.5" cy="8" r="1.3" />
      <circle cx="8" cy="8" r="1.3" />
      <circle cx="12.5" cy="8" r="1.3" />
    </svg>
  ),
  markdown: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.4}>
      <rect x="1.5" y="3.5" width="13" height="9" rx="1.5" />
      <path d="M4 10V6l1.6 2 1.6-2v4M10.5 6v4M9 8.5l1.5 1.5 1.5-1.5" />
    </svg>
  ),
  columns: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.4}>
      <rect x="1.5" y="3" width="13" height="10" rx="1.5" />
      <path d="M8 3v10" />
    </svg>
  ),
  pencil: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <path d="M11 2.2a1.9 1.9 0 0 1 2.7 2.7l-8.2 8.2-3.4.7.7-3.4z" />
      <path d="M10.2 3l2.7 2.7" />
    </svg>
  ),
  copy: ({ size, ...p }: IconProps) => (
    <svg {...stroke(size, p)} strokeWidth={1.5}>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M10.5 3.5v-.5A1.5 1.5 0 0 0 9 1.5H3.5A1.5 1.5 0 0 0 2 3v5.5a1.5 1.5 0 0 0 1.5 1.5h.5" />
    </svg>
  ),
};
