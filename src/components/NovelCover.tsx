import { useState, type CSSProperties } from 'react';

export interface Cover {
  title: string;
  cover: { bg: string; fg: string; deco: string };
  /** Cover picture; the typographic cover shows while it loads or when it fails. */
  image?: string | null;
}

/** The book's cover picture over a typographic placeholder cover. */
export function NovelCover({ novel, w, h, fs, style }: { novel: Cover; w: number | string; h: number | string; fs: number; style?: CSSProperties }) {
  const { bg, fg, deco } = novel.cover;
  const [failed, setFailed] = useState<string | null>(null);
  const image = novel.image && failed !== novel.image ? novel.image : null;
  return (
    <div className="cover" style={{ width: w, height: h, background: bg, color: fg, ...style }}>
      <div className="cover__deco" style={{ background: deco }} />
      <div className="cover__title" style={{ fontSize: fs }}>{novel.title}</div>
      {image && <img className="cover__img" src={image} alt="" loading="lazy" onError={() => setFailed(image)} />}
    </div>
  );
}
