import { findNovel } from '../data/repository';
import { useT } from '../i18n';
import { useApp, type Tab } from '../store/AppStore';
import { BookOpenIcon, GridIcon, HeartIcon, HomeIcon, PlayIcon, UserIcon } from './Icons';
import { NovelCover } from './NovelCover';
import { useContinue } from '../lib/useContinue';
import { localCover } from '../lib/localBook';

const initialsOf = (name?: string) => (name ? name.split(' ').map(w => w[0]).slice(0, 2).join('').toUpperCase() : 'G');


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
  const online = (app.last && findNovel(app.last.id)) ?? null;
  // Whatever was read last: an imported book or an online novel.
  const local = app.lastLocal;
  const last = local
    ? { title: local.title, cover: localCover(local.title), sub: t('book.part', { n: local.index + 1, total: local.total }), pct: (local.index + 1) / Math.max(local.total, 1) }
    : online && app.last
      ? { title: online.title, cover: online.cover, sub: t('common.chapter', { n: app.last.ch }), pct: app.last.ch / online.chapters }
      : null;
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
      {last && (
        <div className="side__continue">
          <div className="kicker">{t('nav.continueReading')}</div>
          <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
            <NovelCover novel={last} w={46} h={66} fs={8} />
            <div style={{ minWidth: 0, flex: 1, display: 'flex', flexDirection: 'column', gap: 5 }}>
              <div style={{ fontWeight: 700, fontSize: 13, lineHeight: 1.25 }}>{last.title}</div>
              <div className="muted" style={{ fontSize: 11 }}>{last.sub}</div>
              <div className="progress"><div style={{ width: `${Math.max(4, Math.min(100, Math.round(last.pct * 100)))}%` }} /></div>
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
