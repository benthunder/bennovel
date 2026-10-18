import type { CSSProperties, ReactNode } from 'react';
import { useId } from 'react';
import { CheckIcon, SearchIcon, XIcon } from './Icons';

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

