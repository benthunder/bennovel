import { useState } from 'react';
import { DICTIONARY, DICT_SUGGEST, TRANSLATE_STYLES } from '../data/mock';
import { getContentLangs } from '../data/repository';
import { CONTENT_LANGS, type ContentLang, type DictEntry, type GlossaryTerm, type ReplaceRule, type TranslateStyle } from '../data/types';
import { useI18n } from '../i18n';
import { DEFAULT_READER_PREFS, READER_THEMES, useApp, type ReaderPrefs } from '../store/AppStore';
import { BookIcon, SparklesIcon, XIcon } from './Icons';
import { Checkbox, Segmented } from './ui';
import { useLayout } from '../lib/useLayout';

export interface Translation { lang: ContentLang; style: TranslateStyle; dict: boolean }

const FONT_MIN = 14, FONT_MAX = 26;

/**
 * Reading settings and the (simulated) AI dictionary / translator.
 * Phone: bottom sheet over the chapter. Tablet and desktop: a side panel the chapter makes room for.
 */
export function ReaderTools({ glossary, lang, rules, onRulesChange, onClose, onTranslate }: {
  glossary: Record<string, GlossaryTerm>;
  /** Language of the text on screen; replace rules are kept per language. */
  lang: ContentLang;
  rules: ReplaceRule[];
  onRulesChange: (rules: ReplaceRule[]) => void;
  onClose: () => void;
  onTranslate: (t: Translation) => void;
}) {
  const { t } = useI18n();
  const lay = useLayout();
  const [tab, setTab] = useState<'reading' | 'replace' | 'ai'>('reading');

  return (
    <div style={{
      position: 'absolute', top: 0, right: 0, bottom: 0, left: lay.wide ? 'auto' : 0, width: lay.wide ? lay.toolsW : 'auto',
      zIndex: 50, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', color: 'var(--color-text)'
    }}>
      {lay.isPhone && <div className="scrim" onClick={onClose} />}
      <div className={`sheet${lay.wide ? ' sheet--side' : ''}`} role="dialog" aria-modal={lay.isPhone} aria-label={t('reader.tools')}>
        <div style={{ padding: `${lay.v('10px', 'calc(env(safe-area-inset-top, 0px) + 42px)', '22px')} 20px 0` }}>
          {lay.isPhone && <div className="sheet__grip" />}
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
            <Segmented value={tab} onChange={setTab} style={{ flex: 1 }} optStyle={{ padding: 10 }}
              options={[{ label: t('tools.reading'), value: 'reading' }, { label: t('tools.replace'), value: 'replace' }, { label: t('tools.ai'), value: 'ai' }]} />
            <button className="btn btn-icon btn-secondary" style={{ width: 40, height: 40 }} aria-label={t('common.close')} onClick={onClose}><XIcon size={16} /></button>
          </div>
        </div>
        <div className="nx-scroll" style={{ overflowY: 'auto', padding: '0 20px calc(env(safe-area-inset-bottom) + 30px)', display: 'flex', flexDirection: 'column', gap: 18 }}>
          {tab === 'reading' && <ReadingTab />}
          {tab === 'replace' && <ReplaceTab lang={lang} rules={rules} onChange={onRulesChange} />}
          {tab === 'ai' && <AiTab glossary={glossary} onTranslate={onTranslate} />}
        </div>
      </div>
    </div>
  );
}

const Label = ({ children, right }: { children: React.ReactNode; right?: React.ReactNode }) => (
  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 13, fontWeight: 700, marginBottom: 8 }}>
    <span>{children}</span>{right !== undefined && <span className="muted">{right}</span>}
  </div>
);

function ReadingTab() {
  const { t } = useI18n();
  const app = useApp();
  const p = app.readerPrefs;
  const set = (patch: Partial<ReaderPrefs>) => app.setReaderPrefs(prev => ({ ...prev, ...patch }));
  const clampFont = (n: number) => Math.min(FONT_MAX, Math.max(FONT_MIN, n));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <div>
        <Label right={`${p.fontSize}px`}>{t('tools.fontSize')}</Label>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <button className="btn btn-secondary btn-icon" style={{ width: 40, height: 40, fontSize: 13, flex: 'none' }} onClick={() => set({ fontSize: clampFont(p.fontSize - 1) })}>A−</button>
          <input type="range" min={FONT_MIN} max={FONT_MAX} step={1} value={p.fontSize} aria-label={t('tools.fontSize')} onChange={e => set({ fontSize: +e.target.value })} />
          <button className="btn btn-secondary btn-icon" style={{ width: 40, height: 40, fontSize: 18, flex: 'none' }} onClick={() => set({ fontSize: clampFont(p.fontSize + 1) })}>A+</button>
        </div>
      </div>
      <div>
        <Label>{t('tools.background')}</Label>
        <div style={{ display: 'flex', gap: 14 }}>
          {(Object.keys(READER_THEMES) as (keyof typeof READER_THEMES)[]).map(k => (
            <button key={k} aria-pressed={p.theme === k} onClick={() => set({ theme: k })}
              style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6, border: 'none', background: 'none', cursor: 'pointer', font: 'inherit', fontSize: 11, color: 'var(--color-text)' }}>
              <span className="display" style={{ width: 50, height: 50, borderRadius: '50%', background: READER_THEMES[k].bg, color: READER_THEMES[k].fg, display: 'grid', placeItems: 'center', fontSize: 17, boxShadow: `0 0 0 ${p.theme === k ? 3 : 0}px var(--color-accent), var(--shadow-sm)` }}>Aa</span>
              {t(`theme.${k}`)}
            </button>
          ))}
        </div>
      </div>
      <div>
        <Label right={p.lineH}>{t('tools.lineSpacing')}</Label>
        <input type="range" min={1.3} max={2.3} step={0.05} value={p.lineH} aria-label={t('tools.lineSpacing')} onChange={e => set({ lineH: +(+e.target.value).toFixed(2) })} />
      </div>
      <div>
        <Label>{t('tools.gap')}</Label>
        <Segmented value={p.gap} onChange={gap => set({ gap })} options={(['tight', 'normal', 'airy'] as const).map(v => ({ label: t(`gap.${v}`), value: v }))} />
      </div>
      <div>
        <Label>{t('tools.margins')}</Label>
        <Segmented value={p.margin} onChange={margin => set({ margin })} options={(['narrow', 'normal', 'wide'] as const).map(v => ({ label: t(`margin.${v}`), value: v }))} />
      </div>
      <div>
        <Label>{t('tools.alignment')}</Label>
        <Segmented value={p.align} onChange={align => set({ align })} options={(['left', 'justify'] as const).map(v => ({ label: t(`align.${v}`), value: v }))} />
      </div>
      <button className="btn btn-ghost" style={{ alignSelf: 'flex-start' }} onClick={() => app.setReaderPrefs(DEFAULT_READER_PREFS)}>{t('tools.reset')}</button>
    </div>
  );
}

/** The reader's own "find → replace" list for this novel in the language on screen. */
function ReplaceTab({ lang, rules, onChange }: { lang: ContentLang; rules: ReplaceRule[]; onChange: (rules: ReplaceRule[]) => void }) {
  const { t } = useI18n();
  const [find, setFind] = useState('');
  const [replace, setReplace] = useState('');
  const langName = getContentLangs().find(l => l.code === lang)?.native ?? lang;

  const add = () => {
    const f = find.trim();
    if (!f) return;
    // Adding a word that already has a rule changes that rule instead of stacking a second one.
    const i = rules.findIndex(r => r.find === f);
    onChange(i >= 0 ? rules.map((r, j) => (j === i ? { find: f, replace } : r)) : [...rules, { find: f, replace }]);
    setFind('');
    setReplace('');
  };

  return (
    <div style={panel}>
      <div>
        <div className="display" style={{ fontSize: 18 }}>{t('replace.title')}</div>
        <div className="muted" style={{ fontSize: 12, marginTop: 4 }}>{t('replace.sub', { lang: langName })}</div>
      </div>
      <form style={{ display: 'flex', flexDirection: 'column', gap: 8 }} onSubmit={e => { e.preventDefault(); add(); }}>
        <input className="input" style={{ height: 42, background: 'var(--color-bg)' }} aria-label={t('replace.find')} placeholder={t('replace.find')} value={find} onChange={e => setFind(e.target.value)} maxLength={200} />
        <input className="input" style={{ height: 42, background: 'var(--color-bg)' }} aria-label={t('replace.with')} placeholder={t('replace.with')} value={replace} onChange={e => setReplace(e.target.value)} maxLength={200} />
        <button type="submit" className="btn btn-primary" style={{ height: 42 }} disabled={!find.trim()}>{t('replace.add')}</button>
      </form>
      {rules.length === 0
        ? <div className="muted" style={{ fontSize: 12 }}>{t('replace.empty')}</div>
        : rules.map(r => (
          <div key={r.find} style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 13, padding: '8px 8px 8px 12px', borderRadius: 16, background: 'var(--color-bg)' }}>
            <span style={{ flex: 1, minWidth: 0, overflowWrap: 'anywhere' }}>
              <span style={{ fontWeight: 700 }}>{r.find}</span> → {r.replace || <span className="muted">{t('replace.removed')}</span>}
            </span>
            <button className="btn btn-icon btn-secondary" style={{ width: 32, height: 32, flex: 'none' }} aria-label={t('replace.delete', { w: r.find })}
              onClick={() => onChange(rules.filter(x => x !== r))}><XIcon size={14} /></button>
          </div>
        ))}
    </div>
  );
}

const panel: React.CSSProperties = { borderRadius: 30, background: 'var(--color-surface)', padding: 16, display: 'flex', flexDirection: 'column', gap: 12 };
const panelIcon = (bg: string, fg: string): React.CSSProperties => ({ width: 34, height: 34, borderRadius: '50%', background: bg, color: fg, display: 'grid', placeItems: 'center' });

function AiTab({ glossary: terms, onTranslate }: { glossary: Record<string, GlossaryTerm>; onTranslate: (t: Translation) => void }) {
  const { t } = useI18n();
  const app = useApp();
  const currentLang = app.top.s === 'reader' ? app.top.lang : 'en';
  const [word, setWord] = useState('');
  const [result, setResult] = useState<(DictEntry & { word: string }) | null>(null);
  const [trLang, setTrLang] = useState<ContentLang>(currentLang === 'vi' ? 'en' : 'vi');
  const [style, setStyle] = useState<TranslateStyle>('Natural');
  const [applyDict, setApplyDict] = useState(true);
  const glossary = Object.entries(terms);

  // Simulated lookup until the dictionary API exists.
  const lookup = (w: string) => {
    const key = w.trim().toLowerCase();
    if (!key) return;
    setWord(key);
    setResult({ word: key, ...(DICTIONARY[key] ?? { pos: t('dict.aiPos'), def: t('dict.aiDef', { w: key }), ex: t('dict.aiTip') }) });
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={panel}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={panelIcon('var(--color-accent-2-300)', 'var(--color-accent-2-900)')}><BookIcon size={17} /></span>
          <div className="display" style={{ fontSize: 18 }}>{t('dict.title')}</div>
        </div>
        <form style={{ display: 'flex', gap: 8 }} onSubmit={e => { e.preventDefault(); lookup(word); }}>
          <input className="input" style={{ height: 42, background: 'var(--color-bg)' }} placeholder={t('dict.placeholder')} value={word} onChange={e => setWord(e.target.value)} />
          <button type="submit" className="btn btn-primary" style={{ height: 42, flex: 'none' }}>{t('dict.lookup')}</button>
        </form>
        <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          {DICT_SUGGEST.map(w => (
            <button key={w} className="tag tag-neutral" style={{ border: 'none', cursor: 'pointer', font: 'inherit', fontSize: 12 }} onClick={() => lookup(w)}>{w}</button>
          ))}
        </div>
        {result && (
          <div style={{ background: 'var(--color-bg)', borderRadius: 22, padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 4 }}>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>{result.word}</span>
              <span style={{ fontSize: 12, fontStyle: 'italic', color: 'var(--color-accent-2-700)' }}>{result.pos}</span>
            </div>
            <div style={{ fontSize: 13 }}>{result.def}</div>
            <div className="muted" style={{ fontSize: 12, fontStyle: 'italic' }}>{result.ex}</div>
          </div>
        )}
        <div style={{ fontSize: 12, fontWeight: 700, marginTop: 4 }}>{t('dict.glossary')}</div>
        {glossary.map(([key, g]) => (
          <div key={key} style={{ display: 'flex', justifyContent: 'space-between', gap: 10, fontSize: 12, padding: '8px 12px', borderRadius: 16, background: 'var(--color-bg)' }}>
            <span style={{ fontWeight: 700 }}>{g.en ?? key}</span>
            <span className="muted">{CONTENT_LANGS.filter(l => l !== 'en').map(l => g[l]).filter(Boolean).join(' · ')}</span>
          </div>
        ))}
      </div>

      <div style={panel}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={panelIcon('var(--color-accent-300)', 'var(--color-accent-900)')}><SparklesIcon size={17} /></span>
          <div className="display" style={{ fontSize: 18 }}>{t('tr.title')}</div>
        </div>
        <div>
          <div style={{ fontSize: 12, fontWeight: 700, marginBottom: 6 }}>{t('tr.to')}</div>
          <Segmented value={trLang} onChange={setTrLang} style={{ background: 'var(--color-bg)' }} options={getContentLangs().map(l => ({ label: l.native, value: l.code }))} />
        </div>
        <div>
          <div style={{ fontSize: 12, fontWeight: 700, marginBottom: 6 }}>{t('tr.style')}</div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
            {TRANSLATE_STYLES.map(s => (
              <button key={s} className={`chip chip--outline${style === s ? ' chip--active' : ''}`} style={{ height: 38, background: style === s ? undefined : 'var(--color-surface)' }}
                aria-pressed={style === s} onClick={() => setStyle(s)}>{t(`tr.style.${s}`)}</button>
            ))}
          </div>
          <div className="muted" style={{ fontSize: 12, marginTop: 8 }}>{t(`tr.desc.${style}`)}</div>
        </div>
        <Checkbox checked={applyDict} onChange={() => setApplyDict(a => !a)} style={{ padding: '10px 12px', borderRadius: 20, background: 'var(--color-bg)' }}>
          <span style={{ flex: 1 }}>
            <span style={{ display: 'block', fontSize: 13, fontWeight: 700 }}>{t('tr.applyDict')}</span>
            <span className="muted" style={{ fontSize: 11 }}>{t('tr.applyDictSub', { n: glossary.length })}</span>
          </span>
        </Checkbox>
        <button className="btn btn-primary" style={{ height: 48, fontSize: 15 }} onClick={() => onTranslate({ lang: trLang, style, dict: applyDict })}>{t('tr.run')}</button>
      </div>
    </div>
  );
}
