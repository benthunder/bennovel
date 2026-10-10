import { useEffect, useLayoutEffect, useRef, useState } from 'react';
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

/** The device clock, refreshed on each new minute. */
function useClock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let id: ReturnType<typeof setTimeout>;
    const tick = () => { const d = new Date(); setNow(d); id = setTimeout(tick, 60_000 - d.getSeconds() * 1000 - d.getMilliseconds() + 50); };
    tick();
    return () => clearTimeout(id);
  }, []);
  return now;
}

interface BatteryLike extends EventTarget { level: number; charging: boolean }

/** Battery level 0–1 and charging state, or null where the WebView doesn't expose it (iOS, Firefox). */
function useBattery() {
  const [state, setState] = useState<{ level: number; charging: boolean } | null>(null);
  useEffect(() => {
    const get = (navigator as Navigator & { getBattery?: () => Promise<BatteryLike> }).getBattery;
    if (!get) return;
    let battery: BatteryLike | null = null, alive = true;
    const update = () => battery && setState({ level: battery.level, charging: battery.charging });
    get.call(navigator).then(b => {
      if (!alive) return;
      battery = b; update();
      b.addEventListener('levelchange', update);
      b.addEventListener('chargingchange', update);
    }).catch(() => {});
    return () => {
      alive = false;
      battery?.removeEventListener('levelchange', update);
      battery?.removeEventListener('chargingchange', update);
    };
  }, []);
  return state;
}

/** A small status line under the page, in the page's own colours: the time and the battery. */
export function ReaderStatusBar({ theme }: { theme: ReaderThemeStyle }) {
  const lay = useLayout();
  const now = useClock();
  const battery = useBattery();
  const pct = battery ? Math.round(battery.level * 100) : null;
  return (
    <div data-testid="reader-status"
      style={{
        flex: 'none', display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12,
        padding: `4px ${lay.readerBarPx}px calc(env(safe-area-inset-bottom, 0px) + 6px)`,
        background: theme.bg, color: `color-mix(in srgb, ${theme.fg} 62%, transparent)`, fontSize: 11, fontWeight: 600, lineHeight: '16px',
        fontVariantNumeric: 'tabular-nums', userSelect: 'none'
      }}>
      <time dateTime={now.toISOString()}>{now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
      {pct !== null && (
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }} aria-label={`${pct}%`}>
          {battery!.charging && <svg width="7" height="10" viewBox="0 0 7 10" aria-hidden="true"><path d="M4.5 0 0 5.5h3L2.5 10 7 4.5H4z" fill="currentColor" /></svg>}
          {pct}%
          <svg width="20" height="10" viewBox="0 0 20 10" aria-hidden="true">
            <rect x=".5" y=".5" width="16" height="9" rx="2" fill="none" stroke="currentColor" />
            <rect x="17.5" y="3" width="2" height="4" rx="1" fill="currentColor" />
            <rect x="2" y="2" width={Math.max(1, 13 * battery!.level)} height="6" rx="1" fill="currentColor" />
          </svg>
        </span>
      )}
    </div>
  );
}
