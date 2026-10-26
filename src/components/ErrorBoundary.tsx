import { Component, type ErrorInfo, type ReactNode } from 'react';
import { load } from '../lib/storage';

/**
 * Catches a render error so the app shows what broke instead of an empty page
 * (React unmounts the whole tree on an uncaught error). Works outside the i18n and
 * store providers, so it picks its language itself.
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null; info: string }> {
  state = { error: null as Error | null, info: '' };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('BenNovel crashed', error, info.componentStack);
    this.setState({ info: info.componentStack ?? '' });
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;
    return <CrashScreen error={error} detail={info} onRetry={() => this.setState({ error: null, info: '' })} />;
  }
}

const COPY = {
  en: { title: 'Something went wrong', body: 'BenNovel hit an error and could not show this screen. Your books are safe.', retry: 'Try again', reload: 'Restart app', copy: 'Copy error', copied: 'Copied' },
  vi: { title: 'Đã có lỗi xảy ra', body: 'BenNovel gặp lỗi và không hiển thị được màn hình này. Sách của bạn vẫn an toàn.', retry: 'Thử lại', reload: 'Khởi động lại', copy: 'Sao chép lỗi', copied: 'Đã sao chép' }
};

function crashLang(): keyof typeof COPY {
  const saved = load<string | null>('uiLang', null);
  if (saved === 'en' || saved === 'vi') return saved;
  return navigator.language.toLowerCase().startsWith('vi') ? 'vi' : 'en';
}

function CrashScreen({ error, detail, onRetry }: { error: Error; detail: string; onRetry: () => void }) {
  const c = COPY[crashLang()];
  const report = [`${error.name}: ${error.message}`, error.stack, detail].filter(Boolean).join('\n\n');
  const copy = (e: { currentTarget: HTMLButtonElement }) => {
    const btn = e.currentTarget;
    navigator.clipboard?.writeText(report).then(() => { btn.textContent = c.copied; }, () => {});
  };
  return (
    <div className="crash" role="alert">
      <h2 style={{ margin: '0 0 8px' }}>{c.title}</h2>
      <p className="muted" style={{ margin: '0 0 16px' }}>{c.body}</p>
      <pre className="crash__log">{report}</pre>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10, marginTop: 16 }}>
        <button className="btn btn-primary" onClick={onRetry}>{c.retry}</button>
        <button className="btn btn-secondary" onClick={() => location.reload()}>{c.reload}</button>
        <button className="btn btn-secondary" onClick={copy}>{c.copy}</button>
      </div>
    </div>
  );
}
