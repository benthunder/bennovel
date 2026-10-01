// Lucide icons (lucide.dev) at the Organic system's 2.75 stroke width.
import type { CSSProperties, ReactNode } from 'react';

interface IconProps { size?: number; fill?: string; strokeWidth?: number; style?: CSSProperties }

const make = (paths: ReactNode, defaults: Partial<IconProps> = {}) =>
  function Icon({ size = 20, fill = defaults.fill ?? 'none', strokeWidth = defaults.strokeWidth ?? 2.75, style }: IconProps) {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill={fill} stroke="currentColor" strokeWidth={strokeWidth}
        strokeLinecap="round" strokeLinejoin="round" style={style} aria-hidden="true">
        {paths}
      </svg>
    );
  };

export const BookOpenIcon = make(<><path d="M12 7v14" /><path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z" /></>);
export const SearchIcon = make(<><circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" /></>);
export const XIcon = make(<><path d="M18 6 6 18" /><path d="m6 6 12 12" /></>);
export const ChevronLeftIcon = make(<path d="m15 18-6-6 6-6" />);
export const ChevronRightIcon = make(<path d="m9 18 6-6-6-6" />);
export const HeartIcon = make(<path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z" />);
export const PlayIcon = make(<polygon points="6 3 20 12 6 21 6 3" />, { fill: 'currentColor' });
export const LanguagesIcon = make(<><path d="m5 8 6 6" /><path d="m4 14 6-6 2-3" /><path d="M2 5h12" /><path d="M7 2h1" /><path d="m22 22-5-10-5 10" /><path d="M14 18h6" /></>);
export const LogOutIcon = make(<><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" /><polyline points="16 17 21 12 16 7" /><line x1="21" x2="9" y1="12" y2="12" /></>);
export const SlidersIcon = make(<><path d="M20 7h-9" /><path d="M14 17H5" /><circle cx="17" cy="17" r="3" /><circle cx="7" cy="7" r="3" /></>);
export const SparklesIcon = make(<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z" />);
export const BookIcon = make(<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20" />);
export const CheckIcon = make(<path d="M20 6 9 17l-5-5" />, { strokeWidth: 3.5 });
export const HomeIcon = make(<><path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8" /><path d="M3 10a2 2 0 0 1 .709-1.528l7-5.999a2 2 0 0 1 2.582 0l7 5.999A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></>);
export const GridIcon = make(<><rect width="7" height="7" x="3" y="3" rx="2" /><rect width="7" height="7" x="14" y="3" rx="2" /><rect width="7" height="7" x="14" y="14" rx="2" /><rect width="7" height="7" x="3" y="14" rx="2" /></>);
export const UserIcon = make(<><path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2" /><circle cx="12" cy="7" r="4" /></>);
export const GlobeIcon = make(<><circle cx="12" cy="12" r="10" /><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" /><path d="M2 12h20" /></>);
