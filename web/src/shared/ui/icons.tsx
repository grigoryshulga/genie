import type { SVGProps } from "react";

export type IconProps = SVGProps<SVGSVGElement> & { size?: number };
export const stroke = (size = 14, p: IconProps) => ({ width: size, height: size, viewBox: "0 0 16 16", fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true, ...p });

export const Icon = {
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
   * genie mark: the logo in the sidebar and on public pages. The compound path
   * is the smoke-shaped lowercase g of docs/assets/brand/genie-mark.svg, filled
   * with the surrounding color (the tile behind it sets the periwinkle).
   */
  mark: ({ size = 16, ...p }: IconProps) => (
    <svg width={size} height={size} viewBox="300 160 500 570" fill="currentColor" aria-hidden="true" {...p}>
      <path
        fillRule="evenodd"
        d="M667 278C710 293 762 260 767 237C773 212 750 188 726 190C699 190 691 207 680 224C667 247 651 253 625 248C567 233 525 234 483 246C421 264 394 305 391 347C388 397 417 427 469 443L507 452C467 468 415 482 378 513C342 543 326 573 342 609C363 648 411 662 461 662C489 662 516 660 535 657C539 656 537 652 533 650C516 638 487 628 461 618C430 608 411 589 414 568C415 558 417 550 420 542C438 514 477 491 509 476C503 495 519 507 544 518C575 532 632 545 649 569C671 598 653 623 626 644C600 664 549 677 509 687C500 689 501 696 511 695C580 687 652 666 689 628C716 596 712 563 688 539C662 512 608 497 570 484C546 476 536 470 551 465C599 452 644 429 671 400C695 374 700 337 686 308C680 295 673 285 667 278ZM614 343C615 302 593 275 559 270C521 265 489 291 480 326C469 365 479 402 509 416C540 432 576 417 596 393C607 380 613 361 614 343Z"
      />
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
};
