import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/be-vietnam-pro/vietnamese-400.css';
import '@fontsource/be-vietnam-pro/vietnamese-500.css';
import '@fontsource/be-vietnam-pro/vietnamese-600.css';
import '@fontsource/be-vietnam-pro/vietnamese-700.css';
import '@fontsource/be-vietnam-pro/vietnamese-800.css';
import '@fontsource/be-vietnam-pro/latin-400.css';
import '@fontsource/be-vietnam-pro/latin-500.css';
import '@fontsource/be-vietnam-pro/latin-600.css';
import '@fontsource/be-vietnam-pro/latin-700.css';
import '@fontsource/be-vietnam-pro/latin-800.css';
// Reader faces (serif and mono), with full Vietnamese diacritics; the app's sans is Be Vietnam Pro.
import '@fontsource/noto-serif/vietnamese-400.css';
import '@fontsource/noto-serif/vietnamese-700.css';
import '@fontsource/noto-serif/latin-400.css';
import '@fontsource/noto-serif/latin-700.css';
import '@fontsource/jetbrains-mono/vietnamese-400.css';
import '@fontsource/jetbrains-mono/vietnamese-700.css';
import '@fontsource/jetbrains-mono/latin-400.css';
import '@fontsource/jetbrains-mono/latin-700.css';
import './styles/organic.css';
import './styles/app.css';
import './styles/motion.css';
import App from './App';
import { ErrorBoundary } from './components/ErrorBoundary';
import { I18nProvider } from './i18n';
import { AppStoreProvider } from './store/AppStore';

// Desktop Linux runs in WebKitGTK, which often paints on the CPU (old GPUs get no
// acceleration). motion.css and app.css keep that engine to flat colours and short shadows.
if (/Linux/.test(navigator.userAgent) && !/Android|Chrome|Firefox/.test(navigator.userAgent)) {
  document.documentElement.dataset.engine = 'webkitgtk';
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ErrorBoundary>
      <I18nProvider>
        <AppStoreProvider>
          <App />
        </AppStoreProvider>
      </I18nProvider>
    </ErrorBoundary>
  </StrictMode>
);
