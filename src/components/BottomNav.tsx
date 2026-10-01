import { useT } from '../i18n';
import { useApp, type Tab } from '../store/AppStore';
import { GridIcon, HeartIcon, HomeIcon, PlayIcon, UserIcon } from './Icons';

export function BottomNav() {
  const t = useT();
  const app = useApp();
  const item = (tab: Tab, label: string, icon: React.ReactNode) => (
    <button className="bottom-nav__item" aria-current={app.tab === tab ? 'page' : undefined} onClick={() => app.switchTab(tab)}>
      {icon}{label}
    </button>
  );
  const continueLast = () => (app.last ? app.openChapter(app.last.id, app.last.ch) : app.showToast(t('nav.nothing')));

  return (
    <nav className="bottom-nav">
      {item('home', t('nav.home'), <HomeIcon size={22} />)}
      {item('category', t('nav.category'), <GridIcon size={22} />)}
      <button className="bottom-nav__item bottom-nav__fab" onClick={continueLast}>
        <span className="bottom-nav__fab-circle"><PlayIcon size={20} style={{ marginLeft: 3 }} /></span>
        {t('nav.continue')}
      </button>
      {item('library', t('nav.library'), <HeartIcon size={22} />)}
      {item('profile', t('nav.profile'), <UserIcon size={22} />)}
    </nav>
  );
}
