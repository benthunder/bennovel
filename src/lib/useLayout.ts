import { useSyncExternalStore } from 'react';

// Responsive layout tokens from the design handoff (phone 390, tablet 834×1194 / 1194×834, desktop 1440).
export type Device = 'phone' | 'tablet' | 'desktop';

export const TABLET_MIN = 768;
export const DESKTOP_MIN = 1280;

const subscribe = (cb: () => void) => {
  window.addEventListener('resize', cb);
  return () => window.removeEventListener('resize', cb);
};
const getSize = () => `${window.innerWidth}x${window.innerHeight}`;

export function useLayout() {
  const [w, h] = useSyncExternalStore(subscribe, getSize).split('x').map(Number);
  return layoutFor(w, h);
}

export function layoutFor(w: number, h: number) {
  const device: Device = w < TABLET_MIN ? 'phone' : w < DESKTOP_MIN ? 'tablet' : 'desktop';
  const isPhone = device === 'phone', isTablet = device === 'tablet', isDesktop = device === 'desktop';
  const wide = !isPhone;
  const portrait = isTablet && h > w;
  /** Pick a value per device: phone, tablet, desktop. */
  const v = <T,>(p: T, t: T, d: T): T => (isPhone ? p : isTablet ? t : d);
  const px = v(20, 32, 40);
  const showTop = isDesktop || (isTablet && !portrait);

  return {
    device, isPhone, isTablet, isDesktop, wide, portrait, showTop, v,
    px,
    navLeft: v(0, 96, 248),
    /** Top padding of tabbed screens; phone keeps the notch-aware value from CSS. */
    top: v('calc(var(--safe-top) + 10px)', 'calc(env(safe-area-inset-top, 0px) + 44px)', '32px'),
    topPlus: v('calc(var(--safe-top) + 12px)', 'calc(env(safe-area-inset-top, 0px) + 48px)', '36px'),

    homeH: v(26, 32, 36),
    searchW: v('100%', portrait ? '300px' : '380px', '420px'),
    resultCols: v('minmax(0,1fr)', 'repeat(auto-fill,minmax(320px,1fr))', 'repeat(auto-fill,minmax(340px,1fr))'),
    heroCardPad: v(18, 24, 28),
    heroCardGap: v(16, 24, 28),
    heroCoverW: v(118, 170, 196),
    heroCoverFs: v(17, 22, 25),
    heroTitleFs: v(20, 30, 34),
    heroDescFs: v(12.5, 15, 15.5),
    heroDescMax: v(73, 110, 113),
    colW: v(112, 136, 150),
    colFs: v(15, 17, 18),
    colGap: v(14, 18, 20),

    detailCols: isPhone ? 'minmax(0,1fr)' : portrait ? 'minmax(220px,250px) minmax(0,1fr)' : v('', 'minmax(260px,290px) minmax(0,1fr)', 'minmax(280px,320px) minmax(0,1fr)'),
    detailGap: v(0, 32, 40),
    dCoverMax: v('none', portrait ? '210px' : '190px', '250px'),
    dCoverFs: v(18, 22, 26),
    dTitleFs: v(24, 26, 30),
    synFs: v(14, 15, 16),
    chCols: wide && !portrait ? 'repeat(2,minmax(0,1fr))' : 'minmax(0,1fr)',
    chPreview: wide && !portrait ? 10 : 6,
    relW: v(96, 120, 132),
    relFs: v(13, 15, 16),

    listH: v(26, 30, 34),
    listSearchW: v('100%', portrait ? '100%' : '360px', '400px'),
    listCols: v('minmax(0,1fr)', 'repeat(auto-fill,minmax(320px,1fr))', 'repeat(auto-fill,minmax(340px,1fr))'),

    libH: v(30, 34, 38),
    libCols: v('repeat(3,minmax(0,1fr))', 'repeat(auto-fill,minmax(140px,1fr))', 'repeat(auto-fill,minmax(150px,1fr))'),
    libGap: v('16px 12px', '22px 18px', '26px 20px'),
    libFs: v(13, 16, 17),

    profCols: wide ? (portrait ? 'minmax(240px,270px) minmax(0,1fr)' : v('', 'minmax(280px,320px) minmax(0,1fr)', 'minmax(300px,340px) minmax(0,1fr)')) : 'minmax(0,1fr)',
    histCols: wide && !portrait ? 'repeat(2,minmax(0,1fr))' : 'minmax(0,1fr)',

    readerTop: v('calc(var(--safe-top) + 4px)', 'calc(env(safe-area-inset-top, 0px) + 36px)', '16px'),
    readerPx: v(14, 24, 32),
    readerBarPx: v(20, 32, 40),
    readerMax: v('none', '680px', '720px'),
    readerPadTop: v(22, 36, 44),
    readerH: v(28, 34, 38),
    toolsW: 380
  };
}

export type Layout = ReturnType<typeof layoutFor>;
