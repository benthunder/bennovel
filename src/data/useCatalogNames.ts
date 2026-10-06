import { useMemo } from 'react';
import { useI18n } from '../i18n';
import { categoryName, collectionName } from './repository';
import type { Category, CollectionKey } from './types';

/** Category and collection names in the current interface language. */
export function useCatalogNames() {
  const { uiLang } = useI18n();
  return useMemo(() => ({
    cat: (c: Category) => categoryName(c, uiLang),
    coll: (k: CollectionKey) => collectionName(k, uiLang)
  }), [uiLang]);
}
