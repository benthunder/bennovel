import { useState } from 'react';
import { BookOpenIcon } from '../components/Icons';
import { Segmented, Spinner } from '../components/ui';
import { useI18n } from '../i18n';
import type { MessageKey } from '../i18n/en';
import { useApp } from '../store/AppStore';

const SOCIALS = [
  { p: 'Google', glyph: 'G', bg: 'var(--color-accent-200)', fg: 'var(--color-accent-800)', email: 'lina.hart@gmail.com' },
  { p: 'Facebook', glyph: 'f', bg: 'var(--color-accent-2-200)', fg: 'var(--color-accent-2-800)', email: 'lina.hart@facebook.com' },
  { p: 'iCloud', glyph: '☁', bg: 'var(--color-neutral-300)', fg: 'var(--color-neutral-900)', email: 'lina.hart@icloud.com' }
];

type Field = 'name' | 'email' | 'pw';
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export function LoginScreen() {
  const { t, uiLang, setUiLang } = useI18n();
  const app = useApp();
  const [mode, setMode] = useState<'login' | 'register'>('login');
  const [form, setForm] = useState({ name: '', email: '', pw: '' });
  const [errors, setErrors] = useState<Partial<Record<Field, MessageKey>>>({});

  const login = (provider: string, name: string, email: string) =>
    app.login(provider, name, email, t('auth.welcome', { name: name.split(' ')[0] }));

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const reg = mode === 'register';
    const errs: typeof errors = {};
    if (reg && !form.name.trim()) errs.name = 'auth.err.name';
    if (!EMAIL_RE.test(form.email)) errs.email = 'auth.err.email';
    if (form.pw.length < 6) errs.pw = 'auth.err.pw';
    setErrors(errs);
    if (Object.keys(errs).length) return;
    const name = reg ? form.name.trim() : form.email.split('@')[0].replace(/[._]/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
    login('email', name, form.email);
  };

  const field = (key: Field, label: string, props: React.InputHTMLAttributes<HTMLInputElement>) => (
    <div className="field">
      <label htmlFor={`auth-${key}`}>{label}</label>
      <input id={`auth-${key}`} className="input" style={{ height: 44 }} value={form[key]}
        onChange={e => setForm(f => ({ ...f, [key]: e.target.value }))} aria-invalid={!!errors[key]} {...props} />
      {errors[key] && <div style={{ fontSize: 12, color: 'var(--color-accent-700)', margin: '5px 14px 0' }}>{t(errors[key])}</div>}
    </div>
  );

  return (
    <div className="screen" style={{ overflow: 'hidden', padding: 'calc(var(--safe-top) + 6px) 24px calc(env(safe-area-inset-bottom) + 24px)' }}>
      <form onSubmit={submit} noValidate style={{ height: '100%', display: 'flex', flexDirection: 'column', gap: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{ width: 34, height: 34, borderRadius: '50%', background: 'var(--color-accent)', display: 'grid', placeItems: 'center', color: 'var(--color-bg)' }}>
              <BookOpenIcon size={18} />
            </div>
            <span className="display" style={{ fontSize: 20 }}>BenNovel</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
            <button type="button" className="btn btn-ghost" style={{ fontSize: 12 }} aria-label={t('uiLang.label')} onClick={() => setUiLang(uiLang === 'en' ? 'vi' : 'en')}>
              {uiLang === 'en' ? 'VI' : 'EN'}
            </button>
            <button type="button" className="btn btn-ghost" onClick={app.skipLogin}>{app.canBack ? t('auth.notNow') : t('auth.skip')}</button>
          </div>
        </div>

        <h1 style={{ margin: '10px 0 0', fontSize: 30, maxWidth: 300 }}>{t('auth.headline')}</h1>
        <p className="muted" style={{ margin: '-4px 0 0', fontSize: 14, maxWidth: 290 }}>{t('auth.sub')}</p>

        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: 6, marginTop: 4 }}>
          {SOCIALS.map(s => (
            <button key={s.p} type="button" className="btn btn-secondary" aria-label={t('auth.continueWith', { p: s.p })}
              style={{ height: 48, gap: 5, padding: '0 4px', background: 'var(--color-neutral-100)', fontSize: 12, minWidth: 0 }}
              disabled={!!app.authLoading} onClick={() => login(s.p, 'Lina Hart', s.email)}>
              {app.authLoading === s.p
                ? <Spinner size={18} width={3} />
                : <span style={{ width: 22, height: 22, flex: 'none', borderRadius: '50%', background: s.bg, color: s.fg, display: 'grid', placeItems: 'center', fontFamily: 'var(--font-body)', fontWeight: 800, fontSize: 12 }}>{s.glyph}</span>}
              <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.p}</span>
            </button>
          ))}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontSize: 12, color: 'var(--color-neutral-700)' }}>
          <span style={{ flex: 1, height: 2, borderRadius: 2, background: 'var(--color-neutral-300)' }} />
          {t('auth.orEmail')}
          <span style={{ flex: 1, height: 2, borderRadius: 2, background: 'var(--color-neutral-300)' }} />
        </div>

        <Segmented value={mode} onChange={m => { setMode(m); setErrors({}); }}
          options={[{ label: t('auth.login'), value: 'login' }, { label: t('auth.register'), value: 'register' }]} />

        {mode === 'register' && field('name', t('auth.name'), { placeholder: 'Lina Hart', autoComplete: 'name' })}
        {field('email', t('auth.email'), { type: 'email', placeholder: 'you@example.com', autoComplete: 'email', inputMode: 'email' })}
        {field('pw', t('auth.password'), { type: 'password', placeholder: t('auth.pwPlaceholder'), autoComplete: mode === 'login' ? 'current-password' : 'new-password' })}

        <button type="submit" className="btn btn-primary" style={{ height: 50, fontSize: 16 }} disabled={!!app.authLoading}>
          {app.authLoading === 'email' ? t('auth.signingIn') : mode === 'login' ? t('auth.login') : t('auth.createAccount')}
        </button>
        <p className="muted" style={{ margin: 'auto 0 0', fontSize: 11, textAlign: 'center' }}>{t('auth.terms')}</p>
      </form>
    </div>
  );
}
