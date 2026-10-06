import { getNovel } from '../data/repository';
import { useT } from '../i18n';
import { useApp, type Tab } from '../store/AppStore';
import { BookOpenIcon, GridIcon, HeartIcon, HomeIcon, PlayIcon, UserIcon } from './Icons';
import { NovelCover } from './NovelCover';

const initialsOf = (name?: string) => (name ? name.split(' ').map(w => w[0]).slice(0, 2).join('').toUpperCase() : 'G');

function useContinue() {
  const t = useT();
  const app = useApp();
  return () => (app.last ? app.openChapter(app.last.id, app.last.ch) : app.showToast(t('nav.nothing')));
}

/** Tablet: icon rail on the left with the Continue button on top. */
export function NavRail() {
  const t = useT();
  const app = useApp();
  const continueLast = useContinue();
  const item = (tab: Tab, label: string, icon: React.ReactNode) => (
    <button className="rail__item" aria-current={app.tab === tab ? 'page' : undefined} onClick={() => app.switchTab(tab)}>
      <span className="rail__pill">{icon}</span>{label}
    </button>
  );

  return (
    <nav className="rail">
      <div className="rail__logo"><BookOpenIcon size={20} /></div>
      <button className="rail__continue" onClick={continueLast}>
        <span className="rail__fab"><PlayIcon size={20} style={{ marginLeft: 3 }} /></span>
        {t('nav.continue')}
      </button>
      {item('home', t('nav.home'), <HomeIcon size={22} />)}
      {item('category', t('nav.category'), <GridIcon size={22} />)}
      {item('library', t('nav.library'), <HeartIcon size={22} />)}
      <button className="rail__item" style={{ marginTop: 'auto' }} aria-current={app.tab === 'profile' ? 'page' : undefined} onClick={() => app.switchTab('profile')}>
        <span className="rail__avatar display">{initialsOf(app.user?.name)}</span>{t('nav.profile')}
      </button>
    </nav>
  );
}

/** Desktop: sidebar with labels, the library count and a Continue reading card. */
export function Sidebar() {
  const t = useT();
  const app = useApp();
  const continueLast = useContinue();
  const last = app.last ? getNovel(app.last.id) : null;
  const item = (tab: Tab, label: string, icon: React.ReactNode, extra?: React.ReactNode) => (
    <button className="side__item" aria-current={app.tab === tab ? 'page' : undefined} onClick={() => app.switchTab(tab)}>
      {icon}{label}{extra}
    </button>
  );

  return (
    <nav className="side">
      <div className="side__brand">
        <span className="side__logo"><BookOpenIcon size={19} /></span>
        <span className="display" style={{ fontSize: 21 }}>BenNovel</span>
      </div>
      {item('home', t('nav.home'), <HomeIcon />)}
      {item('category', t('nav.categories'), <GridIcon />)}
      {item('library', t('nav.library'), <HeartIcon />, <span style={{ marginLeft: 'auto', fontSize: 12, opacity: .7 }}>{app.favs.length}</span>)}
      {item('profile', t('nav.profile'), <UserIcon />)}
      {last && app.last && (
        <div className="side__continue">
          <div className="kicker">{t('nav.continueReading')}</div>
          <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
            <NovelCover novel={last} w={46} h={66} fs={8} />
            <div style={{ minWidth: 0, flex: 1, display: 'flex', flexDirection: 'column', gap: 5 }}>
              <div style={{ fontWeight: 700, fontSize: 13, lineHeight: 1.25 }}>{last.title}</div>
              <div className="muted" style={{ fontSize: 11 }}>{t('common.chapter', { n: app.last.ch })}</div>
              <div className="progress"><div style={{ width: `${Math.max(4, Math.round((app.last.ch / last.chapters) * 100))}%` }} /></div>
            </div>
          </div>
          <button className="btn btn-primary btn-block" style={{ height: 42, gap: 8 }} onClick={continueLast}>
            <PlayIcon size={14} />{t('profile.resume')}
          </button>
        </div>
      )}
    </nav>
  );
}
