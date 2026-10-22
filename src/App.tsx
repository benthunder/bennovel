import { BottomNav } from './components/BottomNav';
import { ErrorBoundary } from './components/ErrorBoundary';
import { LangDialog } from './components/LangDialog';
import { NavRail, Sidebar } from './components/SideNav';
import { useLayout } from './lib/useLayout';
import { useOpenedFiles } from './lib/useOpenedFiles';
import { useApp, type Route } from './store/AppStore';
import { LibraryScreen } from './screens/LibraryScreen';
import { ProfileScreen } from './screens/ProfileScreen';
import { LocalBookScreen } from './screens/LocalBookScreen';
import { BookDetailScreen } from './screens/BookDetailScreen';
import { WikiScreen } from './screens/WikiScreen';

function renderRoute(r: Route) {
  switch (r.s) {
    case 'library': return <LibraryScreen />;
    case 'profile': return <ProfileScreen />;
    case 'bookDetail': return <BookDetailScreen novelId={r.novelId} />;
    case 'bookWiki': return <WikiScreen novelId={r.novelId} title={r.title} lang={r.lang} />;
    case 'book': return <LocalBookScreen book={r.book} place={r.place} path={r.path} />;
  }
}

export default function App() {
  const app = useApp();
  const lay = useLayout();
  // Runs before any book is opened: a book opened from the system goes straight to the reader.
  useOpenedFiles();
  const top = app.top;
  // Keyed by stack depth + route, so each pushed screen starts fresh (scroll, search, filters).
  const key = `${app.stack.length}:${JSON.stringify(top)}`;
  const showNav = top.s !== 'book';
  // Tablet gets a navigation rail and desktop a sidebar; content starts to their right.
  const contentLeft = showNav ? lay.navLeft : 0;

  return (
    <div className="app" data-device={lay.device}>
      {/* A screen that crashes shows its error, and the navigation still works around it. */}
      <div key={key} style={{ position: 'absolute', top: 0, bottom: 0, right: 0, left: contentLeft }}>
        <ErrorBoundary>{renderRoute(top)}</ErrorBoundary>
      </div>
      {showNav && (lay.isPhone ? <BottomNav /> : lay.isTablet ? <NavRail /> : <Sidebar />)}
      {app.langDialogOpen && <LangDialog onClose={app.closeLangDialog} />}
      {app.toast && (
        <div className="toast-wrap" style={{ left: contentLeft }}>
          <div className="toast" role="status">{app.toast}</div>
        </div>
      )}
    </div>
  );
}
