import { BottomNav } from './components/BottomNav';
import { LangDialog } from './components/LangDialog';
import { useApp, type Route } from './store/AppStore';
import { DetailScreen } from './screens/DetailScreen';
import { HomeScreen } from './screens/HomeScreen';
import { LibraryScreen } from './screens/LibraryScreen';
import { ListScreen } from './screens/ListScreen';
import { LoginScreen } from './screens/LoginScreen';
import { ProfileScreen } from './screens/ProfileScreen';
import { ReaderScreen } from './screens/ReaderScreen';

function renderRoute(r: Route) {
  switch (r.s) {
    case 'login': return <LoginScreen />;
    case 'home': return <HomeScreen />;
    case 'detail': return <DetailScreen id={r.id} />;
    case 'list': return <ListScreen src={r.src} />;
    case 'library': return <LibraryScreen />;
    case 'profile': return <ProfileScreen />;
    case 'reader': return <ReaderScreen id={r.id} ch={r.ch} lang={r.lang} />;
  }
}

export default function App() {
  const app = useApp();
  const top = app.top;
  // Keyed by stack depth + route, so each pushed screen starts fresh (scroll, search, filters).
  const key = `${app.stack.length}:${JSON.stringify(top)}`;

  return (
    <div className="app">
      <div key={key} style={{ position: 'absolute', inset: 0 }}>{renderRoute(top)}</div>
      {top.s !== 'reader' && top.s !== 'login' && <BottomNav />}
      {app.langDialog && <LangDialog key={JSON.stringify(app.langDialog)} dialog={app.langDialog} />}
      {app.toast && <div className="toast" role="status">{app.toast}</div>}
    </div>
  );
}
