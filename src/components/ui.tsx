import type { CSSProperties, ReactNode } from 'react';
import { useId } from 'react';
import { useI18n } from '../i18n';
import type { Novel } from '../data/types';
import { CheckIcon, SearchIcon, XIcon } from './Icons';
import { NovelCover } from './NovelCover';

export const Spinner = ({ size = 12, width = 2 }: { size?: number; width?: number }) => (
  <span className="spinner" style={{ width: size, height: size, borderWidth: width }} />
);

export function Segmented<T extends string>({ options, value, onChange, style, optStyle }: {
  options: { label: ReactNode; value: T }[];
  value: T;
  onChange: (v: T) => void;
  style?: CSSProperties;
  optStyle?: CSSProperties;
}) {
  const name = useId();
  return (
    <div className="seg" style={{ display: 'flex', ...style }}>
      {options.map(o => (
        <label key={o.value} className="seg-opt" style={{ flex: 1, justifyContent: 'center', ...optStyle }}>
          <input type="radio" name={name} checked={value === o.value} onChange={() => onChange(o.value)} />
          {o.label}
        </label>
      ))}
    </div>
  );
}

export function Checkbox({ checked, onChange, children, size = 24, style }: {
  checked: boolean; onChange: () => void; children: ReactNode; size?: number; style?: CSSProperties;
}) {
  return (
    <label className="check" style={style}>
      <input type="checkbox" checked={checked} onChange={onChange} />
      <span className="check__box" style={{ width: size, height: size }}><CheckIcon size={size - 10} /></span>
      {children}
    </label>
  );
}

export function SearchField({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <div className="search">
      <span className="search__icon"><SearchIcon size={18} /></span>
      <input className="input" type="search" placeholder={placeholder} value={value} onChange={e => onChange(e.target.value)} />
      {value && (
        <button className="search__clear" aria-label="Clear" onClick={() => onChange('')}><XIcon size={14} /></button>
      )}
    </div>
  );
}

export function NovelRow({ novel, onOpen, statusAsTag, coverW = 64, coverH = 92 }: {
  novel: Novel; onOpen: () => void; statusAsTag?: boolean; coverW?: number; coverH?: number;
}) {
  const { t, uiLang } = useI18n();
  return (
    <button className="novel-row" onClick={onOpen}>
      <NovelCover novel={novel} w={coverW} h={coverH} fs={11} />
      <div className="novel-row__body">
        <div className="novel-row__title">{novel.title}</div>
        <div className="muted" style={{ fontSize: 12 }}>{novel.author} · {t(`cat.${novel.cat}`)}</div>
        <div className="novel-row__desc clamp-2">{novel.desc[uiLang]}</div>
        <div className="novel-row__meta">
          <span className="rating">★ {novel.rating}</span>
          <span>{t('common.ch', { n: novel.chapters })}</span>
          {statusAsTag
            ? <span className="tag tag-neutral" style={{ padding: '1px 8px', fontSize: 10 }}>{t(`status.${novel.status}`)}</span>
            : <span>{t(`status.${novel.status}`)}</span>}
        </div>
      </div>
    </button>
  );
}

export function ShelfItem({ novel, onOpen, w, h, fs, showAuthor = true, titleSize = 13 }: {
  novel: Novel; onOpen: () => void; w: number; h: number; fs: number; showAuthor?: boolean; titleSize?: number;
}) {
  return (
    <button className="shelf-item" style={{ width: w }} onClick={onOpen}>
      <NovelCover novel={novel} w={w} h={h} fs={fs} />
      <div className="shelf-item__title clamp-2" style={{ fontSize: titleSize }}>{novel.title}</div>
      {showAuthor && <div className="shelf-item__author muted">{novel.author}</div>}
    </button>
  );
}

export function CoverGrid({ novels, onOpen, showAuthor = true }: { novels: Novel[]; onOpen: (id: number) => void; showAuthor?: boolean }) {
  return (
    <div className="cover-grid">
      {novels.map(n => (
        <button key={n.id} className="shelf-item" style={{ minWidth: 0 }} onClick={() => onOpen(n.id)}>
          <NovelCover novel={n} w="100%" h="auto" fs={13} style={{ aspectRatio: 0.7 }} />
          <div className="shelf-item__title" style={{ fontSize: 12 }}>{n.title}</div>
          {showAuthor && <div className="shelf-item__author muted">{n.author}</div>}
        </button>
      ))}
    </div>
  );
}

export const formatReads = (k: number) => (k >= 1000 ? `${(k / 1000).toFixed(1)}M` : `${k}K`);
