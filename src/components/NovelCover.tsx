import type { CSSProperties } from 'react';

export interface Cover {
  title: string;
  cover: { bg: string; fg: string; deco: string };
}

/** Typographic placeholder cover until real cover art is available. */
export function NovelCover({ novel, w, h, fs, style }: { novel: Cover; w: number | string; h: number | string; fs: number; style?: CSSProperties }) {
  const { bg, fg, deco } = novel.cover;
  return (
    <div className="cover" style={{ width: w, height: h, background: bg, color: fg, ...style }}>
      <div className="cover__deco" style={{ background: deco }} />
      <div className="cover__title" style={{ fontSize: fs }}>{novel.title}</div>
    </div>
  );
}
