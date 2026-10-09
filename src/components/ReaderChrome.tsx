import { useLayoutEffect, useRef, useState } from 'react';
import { useLayout } from '../lib/useLayout';
import type { ReaderThemeStyle } from '../store/AppStore';

/** How far you scroll one way before the header hides or comes back. */
const SCROLL_SLACK = 24;
/** Below this the header always shows. */
const TOP_ZONE = 40;
/** Strip left on screen when the header hides: the progress bar and its padding. */
const BAR_STRIP = 17;

/**
 * Full-screen reading: the header slides away while you scroll down and comes back
 * when you scroll up, reach the top, or tap the page. `pinned` keeps it on screen.
 */
export function useImmersive(pinned: boolean) {
  const [hidden, setHidden] = useState(false);
  const last = useRef(0);
  const anchor = useRef(0);
  const dir = useRef(0);

  const onScroll = (el: HTMLElement) => {
    const top = el.scrollTop, d = Math.sign(top - last.current);
    if (d && d !== dir.current) { dir.current = d; anchor.current = last.current; }
    last.current = top;
    if (top < TOP_ZONE) setHidden(false);
    else if (top - anchor.current > SCROLL_SLACK) setHidden(true);
    else if (anchor.current - top > SCROLL_SLACK) setHidden(false);
  };
  // A tap on the text (not a button, link or selection) shows or hides the header.
  const onTap = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('button, a, input, select, textarea')) return;
    if (window.getSelection()?.toString()) return;
    setHidden(h => !h);
  };

  return { hidden: hidden && !pinned, onScroll, onTap };
}

/**
 * The reader's header over the page, with the chapter progress bar under it.
 * When hidden only the bar stays, at the top of the screen.
 */
export function ReaderChrome({ hidden, theme, readPct, line, onHeight, children }: {
  hidden: boolean;
  theme: ReaderThemeStyle;
  readPct: number;
  line: string;
  onHeight: (h: number) => void;
  children: React.ReactNode;
}) {
  const lay = useLayout();
  const ref = useRef<HTMLDivElement>(null);
  const safeTop = lay.v('var(--safe-top)', 'env(safe-area-inset-top, 0px)', '0px');
  const motion = theme.still ? 'none' : undefined;

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => onHeight(el.offsetHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <div ref={ref} data-testid="reader-chrome" data-hidden={hidden}
      style={{
        position: 'absolute', top: 0, left: 0, right: 0, zIndex: 20, background: theme.bg,
        transform: hidden ? `translateY(calc(-100% + ${safeTop} + ${BAR_STRIP}px))` : 'none',
        transition: motion ?? 'transform .25s ease'
      }}>
      <div inert={hidden} aria-hidden={hidden}
        style={{ padding: `${lay.readerTop} ${lay.readerPx}px 8px`, display: 'flex', alignItems: 'center', gap: 10, opacity: hidden ? 0 : 1, transition: motion ?? 'opacity .2s ease' }}>
        {children}
      </div>
      <div style={{ padding: `4px ${lay.readerBarPx}px 8px` }}>
        <div role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(readPct * 100)}
          style={{ height: 5, borderRadius: 999, background: line, overflow: 'hidden' }}>
          <div style={{ height: '100%', width: `${Math.round(readPct * 100)}%`, background: 'var(--color-accent)', borderRadius: 999 }} />
        </div>
      </div>
    </div>
  );
}
