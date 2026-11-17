import { useI18n } from '../i18n';
import { AI_GENRES, type AiPrefs } from '../lib/ai';

/**
 * A book's genre (forms of address follow it) and the reader's own instructions for
 * the AI, kept with the book and used for every chapter: on "translate the whole book"
 * and in the reader's AI panel alike.
 */
export function AiPrefsFields({ value, onChange, onBlur, idPrefix = 'ai' }: {
  value: AiPrefs;
  onChange: (p: AiPrefs) => void;
  /** Called as the instructions box loses focus (a good time to keep them). */
  onBlur?: () => void;
  idPrefix?: string;
}) {
  const { t } = useI18n();
  return (
    <>
      <div className="field">
        <label htmlFor={`${idPrefix}-genre`}>{t('ai.genre')}</label>
        <select id={`${idPrefix}-genre`} className="input" style={{ height: 40 }} value={value.genre}
          onChange={e => onChange({ ...value, genre: e.target.value as AiPrefs['genre'] })}>
          <option value="">{t('ai.genre.none')}</option>
          {AI_GENRES.map(g => <option key={g} value={g}>{t(`ai.genre.${g}`)}</option>)}
        </select>
        {value.genre && <div className="muted" style={{ fontSize: 12, marginTop: 4 }}>{t(`ai.genreHint.${value.genre}`)}</div>}
      </div>
      <div className="field">
        <label htmlFor={`${idPrefix}-instructions`}>{t('ai.instructions')}</label>
        <textarea id={`${idPrefix}-instructions`} className="input" rows={3} maxLength={2000} style={{ borderRadius: 18, resize: 'vertical', minHeight: 72, padding: '10px 14px' }}
          placeholder={t('ai.instructionsPh')} value={value.instructions} onBlur={onBlur}
          onChange={e => onChange({ ...value, instructions: e.target.value })} />
      </div>
    </>
  );
}
