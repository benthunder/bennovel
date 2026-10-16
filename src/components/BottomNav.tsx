import { useT } from '../i18n';
import { useApp, type Tab } from '../store/AppStore';
import { useContinue } from '../lib/useContinue';
import { HeartIcon, PlayIcon, UserIcon } from './Icons';

export function BottomNav() {
  const t = useT();
  const app = useApp();
  const item = (tab: Tab, label: string, icon: React.ReactNode) => (
    <button className="bottom-nav__item" aria-current={app.tab === tab ? 'page' : undefined} onClick={() => app.switchTab(tab)}>
      {icon}{label}
    </button>
  );
  const continueLast = useContinue();

  return (
    <nav className="bottom-nav">
      {item('library', t('nav.library'), <HeartIcon size={22} />)}
      <button className="bottom-nav__item bottom-nav__fab" onClick={continueLast}>
        <span className="bottom-nav__fab-circle"><PlayIcon size={20} style={{ marginLeft: 3 }} /></span>
        {t('nav.continue')}
      </button>
      {item('profile', t('nav.profile'), <UserIcon size={22} />)}
    </nav>
  );
}
