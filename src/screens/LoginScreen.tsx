import { useState } from 'react';
import { BookOpenIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { Segmented } from '../components/ui';
import { getPopular } from '../data/repository';
import { useLayout } from '../lib/useLayout';
import { useI18n } from '../i18n';
import type { MessageKey } from '../i18n/en';
import { useApp } from '../store/AppStore';
import type { User } from '../data/types';
import { AuthFailure, sendPasswordReset, setNewPassword, signIn, signInWithGoogle, signUp, verifyResetCode } from '../lib/auth';
import { canOpenLocalBooks } from '../lib/localBook';

// Fanned covers at the bottom of the wide-screen panel: [novel id, transform, overlap, z-index].
/** Cover fan of the three most-read novels: [transform, margin-left, z-index]. */
const FAN: [string, number, number][] = [['rotate(-9deg) translateY(14px)', 0, 1], ['rotate(0deg)', -28, 2], ['rotate(8deg) translateY(10px)', -28, 1]];

type Field = 'name' | 'email' | 'pw' | 'code';
/** login/register: the tabs. forgot: ask for a reset email, then its code. newPw: after the reset email. */
type Mode = 'login' | 'register' | 'forgot' | 'newPw';
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const CODE_RE = /^\d{6,10}$/;
const SUBMIT: Record<Mode | 'code', MessageKey> = {
  login: 'auth.login', register: 'auth.createAccount', forgot: 'auth.sendReset', code: 'auth.verifyCode', newPw: 'auth.savePassword'
};

export function LoginScreen({ recovery = false }: { recovery?: boolean }) {
  const { t, uiLang, setUiLang } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const [mode, setMode] = useState<Mode>(recovery ? 'newPw' : 'login');
  const [form, setForm] = useState({ name: '', email: '', pw: '', code: '' });
  const [errors, setErrors] = useState<Partial<Record<Field, MessageKey>>>({});
  const [failure, setFailure] = useState<MessageKey | null>(null);
  /** What the email just sent asks the reader to do. */
  const [notice, setNotice] = useState<string | null>(null);
  /** The reset email went out, so its code is asked for. */
  const [resetSent, setResetSent] = useState(false);
  const [busy, setBusy] = useState(false);

  const go = (m: Mode) => { setMode(m); setErrors({}); setFailure(null); setNotice(null); setResetSent(false); };
  const welcome = (u: User) => t('auth.welcome', { name: u.name.split(' ')[0] });

  /** Checks the fields this step shows; true when they are fine. */
  const check = () => {
    const errs: typeof errors = {};
    if (mode === 'register' && !form.name.trim()) errs.name = 'auth.err.name';
    if (mode !== 'newPw' && !EMAIL_RE.test(form.email.trim())) errs.email = 'auth.err.email';
    if ((mode === 'login' || mode === 'register' || mode === 'newPw') && form.pw.length < 6) errs.pw = 'auth.err.pw';
    if (mode === 'forgot' && resetSent && !CODE_RE.test(form.code.trim())) errs.code = 'auth.err.codeFormat';
    setErrors(errs);
    return !Object.keys(errs).length;
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (busy || !check()) return;
    const email = form.email.trim();
    setBusy(true);
    setFailure(null);
    try {
      if (mode === 'login') {
        const u = await signIn(email, form.pw);
        await app.signedIn(u, welcome(u));
      } else if (mode === 'register') {
        const u = await signUp(form.name.trim(), email, form.pw);
        if (u) await app.signedIn(u, welcome(u));
        else {
          // The account waits for its email to be confirmed; then the reader logs in here.
          go('login');
          setForm(f => ({ ...f, pw: '' }));
          setNotice(t('auth.confirmSent', { email }));
        }
      } else if (mode === 'forgot' && !resetSent) {
        await sendPasswordReset(email);
        setResetSent(true);
        setNotice(t('auth.resetSent', { email }));
      } else if (mode === 'forgot') {
        // Signed in by the code; the app then switches this screen to the new password step.
        await verifyResetCode(email, form.code.trim());
      } else {
        const u = await setNewPassword(form.pw);
        await app.signedIn(u, t('auth.passwordSaved'));
      }
    } catch (err) {
      setFailure(err instanceof AuthFailure ? err.key : 'auth.err.generic');
    } finally {
      setBusy(false);
    }
  };

  /** Google sign-in finishes in the browser; the app hears back through a link (see AppStore). */
  const google = async () => {
    setBusy(true);
    setFailure(null);
    setNotice(null);
    try {
      await signInWithGoogle();
      if (canOpenLocalBooks()) setNotice(t('auth.googleWaiting'));
    } catch (err) {
      setFailure(err instanceof AuthFailure ? err.key : 'auth.err.generic');
    } finally {
      setBusy(false);
    }
  };

  const field = (key: Field, label: string, props: React.InputHTMLAttributes<HTMLInputElement>) => (
    <div className="field">
      <label htmlFor={`auth-${key}`}>{label}</label>
      <input id={`auth-${key}`} className="input" style={{ height: 44 }} value={form[key]}
        onChange={e => setForm(f => ({ ...f, [key]: e.target.value }))} aria-invalid={!!errors[key]} {...props} />
      {errors[key] && <div style={{ fontSize: 12, color: 'var(--color-accent-700)', margin: '5px 14px 0' }}>{t(errors[key])}</div>}
    </div>
  );

  const langToggle = (
    <button type="button" className="btn btn-ghost" style={{ fontSize: 12 }} aria-label={t('uiLang.label')} onClick={() => setUiLang(uiLang === 'en' ? 'vi' : 'en')}>
      {uiLang === 'en' ? 'VI' : 'EN'}
    </button>
  );
  const skip = <button type="button" className="btn btn-ghost" onClick={app.skipLogin}>{app.canBack ? t('auth.notNow') : t('auth.skip')}</button>;

  const formEl = (
    <form onSubmit={submit} noValidate style={{
      position: 'relative', width: '100%', maxWidth: lay.wide ? 400 : 'none', margin: '0 auto', height: lay.wide ? 'auto' : '100%',
      display: 'flex', flexDirection: 'column', gap: 14
    }}>
      {lay.isPhone ? (
        <>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <div style={{ width: 34, height: 34, borderRadius: '50%', background: 'var(--color-accent)', display: 'grid', placeItems: 'center', color: 'var(--color-bg)' }}>
                <BookOpenIcon size={18} />
              </div>
              <span className="display" style={{ fontSize: 20 }}>BenNovel</span>
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>{langToggle}{skip}</div>
          </div>
          <h1 style={{ margin: '10px 0 0', fontSize: 30, maxWidth: 300 }}>{t('auth.headline')}</h1>
          <p className="muted" style={{ margin: '-4px 0 0', fontSize: 14, maxWidth: 290 }}>{t('auth.sub')}</p>
        </>
      ) : (
        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 12 }}>
          <div>
            <h3 style={{ margin: 0, fontSize: 30 }}>{t(`auth.wideTitle.${mode === 'register' ? 'register' : 'login'}`)}</h3>
            <div className="muted" style={{ fontSize: 14, marginTop: 4 }}>{t(`auth.wideSub.${mode === 'register' ? 'register' : 'login'}`)}</div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>{langToggle}{skip}</div>
        </div>
      )}

      {(mode === 'login' || mode === 'register') && (
        <>
          <button type="button" className="btn btn-secondary" disabled={busy} onClick={google}
            style={{ height: 50, gap: 10, marginTop: 4, background: 'var(--color-neutral-100)', fontSize: 15 }}>
            <span style={{ width: 24, height: 24, flex: 'none', borderRadius: '50%', background: 'var(--color-accent-200)', color: 'var(--color-accent-800)', display: 'grid', placeItems: 'center', fontFamily: 'var(--font-body)', fontWeight: 800, fontSize: 13 }}>G</span>
            {t('auth.continueWith', { p: 'Google' })}
          </button>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontSize: 12, color: 'var(--color-neutral-700)' }}>
            <span style={{ flex: 1, height: 2, borderRadius: 2, background: 'var(--color-neutral-300)' }} />
            {t('auth.orEmail')}
            <span style={{ flex: 1, height: 2, borderRadius: 2, background: 'var(--color-neutral-300)' }} />
          </div>
        </>
      )}

      {mode === 'login' || mode === 'register' ? (
        <Segmented value={mode} onChange={go}
          options={[{ label: t('auth.login'), value: 'login' }, { label: t('auth.register'), value: 'register' }]} />
      ) : (
        <div style={{ marginTop: 4 }}>
          <div className="display" style={{ fontSize: 20 }}>{t(mode === 'forgot' ? 'auth.forgotTitle' : 'auth.newPwTitle')}</div>
          <div className="muted" style={{ fontSize: 13, marginTop: 2 }}>{t(mode === 'forgot' ? 'auth.forgotSub' : 'auth.newPwSub')}</div>
        </div>
      )}

      {notice && <div role="status" style={{ fontSize: 13, borderRadius: 18, padding: '10px 14px', background: 'var(--color-accent-2-200)', color: 'var(--color-accent-2-900)' }}>{notice}</div>}

      {mode === 'register' && field('name', t('auth.name'), { placeholder: 'Lina Hart', autoComplete: 'name' })}
      {mode !== 'newPw' && field('email', t('auth.email'), { type: 'email', placeholder: 'you@example.com', autoComplete: 'email', inputMode: 'email', readOnly: mode === 'forgot' && resetSent })}
      {mode === 'forgot' && resetSent && field('code', t('auth.code'), { inputMode: 'numeric', autoComplete: 'one-time-code', placeholder: '123456' })}
      {mode !== 'forgot' && field('pw', t(mode === 'newPw' ? 'auth.newPassword' : 'auth.password'), { type: 'password', placeholder: t('auth.pwPlaceholder'), autoComplete: mode === 'login' ? 'current-password' : 'new-password' })}

      {failure && <div role="alert" style={{ fontSize: 13, color: 'var(--color-accent-700)', margin: '-4px 14px 0' }}>{t(failure)}</div>}

      <button type="submit" className="btn btn-primary" style={{ height: 50, fontSize: 16 }} disabled={busy}>
        {busy ? t('auth.working') : t(SUBMIT[mode === 'forgot' && resetSent ? 'code' : mode])}
      </button>
      {mode === 'login' && <button type="button" className="btn btn-ghost" style={{ alignSelf: 'center', fontSize: 13 }} onClick={() => go('forgot')}>{t('auth.forgot')}</button>}
      {mode === 'forgot' && <button type="button" className="btn btn-ghost" style={{ alignSelf: 'center', fontSize: 13 }} onClick={() => go('login')}>{t('auth.backToLogin')}</button>}
      <p className="muted" style={{ margin: `${lay.wide ? '6px' : 'auto'} 0 0`, fontSize: 11, textAlign: 'center' }}>{t('auth.terms')}</p>
    </form>
  );

  if (lay.isPhone) {
    return (
      <div className="screen" style={{ overflow: 'hidden', padding: 'calc(var(--safe-top) + 6px) 24px calc(env(safe-area-inset-bottom) + 24px)' }}>
        {formEl}
      </div>
    );
  }

  // Tablet and desktop: brand panel beside the form (stacked above it on a portrait tablet).
  const { portrait } = lay;
  return (
    <div className="screen" style={{
      overflow: 'hidden', display: 'grid',
      gridTemplateColumns: portrait ? 'minmax(0,1fr)' : 'minmax(0,1.05fr) minmax(0,1fr)',
      gridTemplateRows: portrait ? 'auto minmax(0,1fr)' : 'minmax(0,1fr)'
    }}>
      <div style={{
        margin: portrait ? 'calc(env(safe-area-inset-top, 0px) + 42px) 18px 0' : lay.v('0', 'calc(env(safe-area-inset-top, 0px) + 42px) 0 18px 18px', '16px 0 16px 16px'),
        borderRadius: 36, background: 'var(--color-accent-2-200)', color: 'var(--color-accent-2-900)', padding: 40,
        display: 'flex', flexDirection: 'column', gap: 24, overflow: 'hidden', minHeight: 0
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{ width: 40, height: 40, borderRadius: '50%', background: 'var(--color-accent)', display: 'grid', placeItems: 'center', color: 'var(--color-bg)' }}>
            <BookOpenIcon size={20} />
          </div>
          <span className="display" style={{ fontSize: 22 }}>BenNovel</span>
        </div>
        <h1 style={{ margin: 0, fontSize: portrait ? 38 : lay.v(30, 44, 52), maxWidth: 520, color: 'var(--color-accent-2-900)', textWrap: 'balance' }}>{t('auth.headline')}</h1>
        <p style={{ margin: '-8px 0 0', fontSize: 16, maxWidth: 440, color: 'var(--color-accent-2-900)' }}>{t('auth.panelSub')}</p>
        {!portrait && (
          <div style={{ marginTop: 'auto', display: 'flex', alignItems: 'flex-end', paddingLeft: 12 }}>
            {getPopular().slice(0, FAN.length).map((novel, i) => { const [transform, ml, z] = FAN[i]; return (
              <div key={novel.id} style={{ width: lay.v(120, 150, 180), flex: 'none', marginLeft: ml, transform, zIndex: z, position: 'relative' }}>
                <NovelCover novel={novel} w="100%" h="auto" fs={20} style={{ aspectRatio: 0.7 }} />
              </div>
            ); })}
          </div>
        )}
      </div>
      <div style={{ minHeight: 0, overflowY: 'auto', padding: lay.v('0', portrait ? '32px 48px' : '48px 48px 32px', '40px 56px'), display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
        {formEl}
      </div>
    </div>
  );
}
