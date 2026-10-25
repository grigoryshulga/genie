// The lane's card list. Cards that do not fit the visible height tuck under the last card that does,
// as a stack; scrolling the lane brings each one out to its place in the list.

import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";

const PEEK = 12; // px between tucked layers
const LAYERS = 3; // layers that show; deeper ones stay hidden behind them

export function StackedList({ children }: { children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);

  // Runs after every render, so a changed list is laid out before the paint.
  useLayoutEffect(() => {
    if (ref.current) stack(ref.current);
  });

  useEffect(() => {
    const list = ref.current;
    if (!list) return;
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => stack(list));
    };
    const resize = new ResizeObserver(() => stack(list));
    resize.observe(list);
    list.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      list.removeEventListener("scroll", onScroll);
    };
  }, []);

  return <div ref={ref} className="g-lane-cards">{children}</div>;
}

function stack(list: HTMLElement) {
  const cards = Array.from(list.children).filter((el): el is HTMLElement => el instanceof HTMLElement && el.classList.contains("bd-card"));
  const bottom = list.scrollTop + list.clientHeight;
  // The stack tucks under the last card that is fully visible.
  const anchor = cards.filter((card) => card.offsetTop + card.offsetHeight <= bottom).at(-1);
  let layer = 0;
  for (const card of cards) {
    // Only a card that is fully in view stays in the list; anything cut by the lane's bottom edge tucks.
    if (!anchor || card.offsetTop + card.offsetHeight <= bottom) {
      card.style.transform = "";
      card.style.zIndex = "";
      card.style.opacity = "";
      card.style.pointerEvents = "";
      continue;
    }
    layer += 1;
    const shown = Math.min(layer, LAYERS);
    // `offsetTop` ignores transforms, so the target stays where the list placed the card.
    card.style.transform = `translateY(${anchor.offsetTop + shown * PEEK - card.offsetTop}px) scale(${1 - 0.02 * shown})`;
    card.style.transformOrigin = "top center";
    card.style.zIndex = String(-shown);
    card.style.opacity = layer > LAYERS ? "0" : "1";
    card.style.pointerEvents = "none";
  }
}
