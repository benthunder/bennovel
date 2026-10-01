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
import App from './App';
import { I18nProvider } from './i18n';
import { AppStoreProvider } from './store/AppStore';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <I18nProvider>
      <AppStoreProvider>
        <App />
      </AppStoreProvider>
    </I18nProvider>
  </StrictMode>
);
