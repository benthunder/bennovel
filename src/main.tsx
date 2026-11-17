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
import './styles/organic.css';
import './styles/app.css';
import './styles/motion.css';
import App from './App';
import { I18nProvider } from './i18n';
import { AppStoreProvider } from './store/AppStore';

// Desktop Linux runs in WebKitGTK, where backdrop-filter is drawn on the CPU and large
// blurs (bars, sheets, the scrim behind dialogs) can stall the page for seconds at a
// time. motion.css swaps the glass for solid colours there.
if (/Linux/.test(navigator.userAgent) && !/Android|Chrome|Firefox/.test(navigator.userAgent)) {
  document.documentElement.dataset.engine = 'webkitgtk';
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <I18nProvider>
      <AppStoreProvider>
        <App />
      </AppStoreProvider>
    </I18nProvider>
  </StrictMode>
);
