import { CoverGrid } from '../components/ui';
import { getNovels } from '../data/repository';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';
import { canOpenLocalBooks, pickLocalBook } from '../lib/localBook';

export function LibraryScreen() {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const favs = getNovels(app.favs);

  const openFile = async () => {
    try {
      const book = await pickLocalBook();
      if (book) app.push({ s: 'book', book });
    } catch (e) {
      console.error(e);
      app.showToast(`${t('book.openFailed')} ${String(e)}`);
    }
  };

  return (
    <div className="screen screen--tabbed" style={{ paddingTop: lay.topPlus, paddingLeft: lay.px, paddingRight: lay.px }}>
      <div className="kicker">{t('library.kicker')}</div>
      <h2 style={{ margin: '0 0 4px', fontSize: lay.libH }}>{t('library.title')}</h2>
      <div className="muted" style={{ fontSize: 13, marginBottom: 18 }}>
        {favs.length === 1 ? t('library.countOne') : t('library.count', { n: favs.length })}
      </div>
      {canOpenLocalBooks() && (
        <button className="btn btn-secondary" style={{ marginBottom: 18 }} onClick={openFile}>{t('book.open')}</button>
      )}
      <CoverGrid novels={favs} onOpen={app.openDetail} cols={lay.libCols} gap={lay.libGap} fs={lay.libFs} />
      {favs.length === 0 && (
        <div className="card" style={{ alignItems: 'flex-start', padding: 22, marginTop: 8, maxWidth: 420 }}>
          <div className="card-title">{t('library.emptyTitle')}</div>
          <p className="card-body">{t('library.emptyBody')}</p>
          <button className="btn btn-primary" onClick={() => app.switchTab('category')}>{t('library.browse')}</button>
        </div>
      )}
    </div>
  );
}
