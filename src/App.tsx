import { BottomNav } from './components/BottomNav';
import { LangDialog } from './components/LangDialog';
import { CatalogLoading } from './components/OfflineNote';
import { NavRail, Sidebar } from './components/SideNav';
import { useCatalog } from './data/useCatalog';
import { useLayout } from './lib/useLayout';
import { useOpenedFiles } from './lib/useOpenedFiles';
import { useApp, type Route } from './store/AppStore';
import { DetailScreen } from './screens/DetailScreen';
import { LibraryScreen } from './screens/LibraryScreen';
import { ProfileScreen } from './screens/ProfileScreen';
import { ReaderScreen } from './screens/ReaderScreen';
import { LocalBookScreen } from './screens/LocalBookScreen';
import { BookDetailScreen } from './screens/BookDetailScreen';

function renderRoute(r: Route) {
  switch (r.s) {
    case 'detail': return <DetailScreen id={r.id} />;
    case 'library': return <LibraryScreen />;
    case 'profile': return <ProfileScreen />;
    case 'reader': return <ReaderScreen id={r.id} ch={r.ch} lang={r.lang} />;
    case 'bookDetail': return <BookDetailScreen novelId={r.novelId} />;
    case 'book': return <LocalBookScreen book={r.book} place={r.place} path={r.path} />;
  }
}

export default function App() {
  const app = useApp();
  const lay = useLayout();
  const { status } = useCatalog();
  // Runs before the catalog has loaded (or without a connection): a book opened from
  // the system goes straight to the reader.
  useOpenedFiles();
  const top = app.top;
  // Online screens wait for the catalog; books on the device don't need it.
  const waiting = status === 'loading' && (top.s === 'detail' || top.s === 'reader');
  // Keyed by stack depth + route, so each pushed screen starts fresh (scroll, search, filters).
  const key = `${app.stack.length}:${JSON.stringify(top)}:${waiting}`;
  const showNav = top.s !== 'reader' && top.s !== 'book';
  // Tablet gets a navigation rail and desktop a sidebar; content starts to their right.
  const contentLeft = showNav ? lay.navLeft : 0;

  return (
    <div className="app" data-device={lay.device}>
      <div key={key} style={{ position: 'absolute', top: 0, bottom: 0, right: 0, left: contentLeft }}>{waiting ? <CatalogLoading /> : renderRoute(top)}</div>
      {showNav && (lay.isPhone ? <BottomNav /> : lay.isTablet ? <NavRail /> : <Sidebar />)}
      {app.langDialog && <LangDialog key={JSON.stringify(app.langDialog)} dialog={app.langDialog} />}
      {app.toast && (
        <div className="toast-wrap" style={{ left: contentLeft }}>
          <div className="toast" role="status">{app.toast}</div>
        </div>
      )}
    </div>
  );
}
