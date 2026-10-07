// Reader tools that have no backend yet: the dictionary lookup and translation styles.
// Everything else now comes from Supabase through ./repository.
import type { DictEntry, TranslateStyle } from './types';

export const DICTIONARY: Record<string, DictEntry> = {
  silvered: { pos: 'verb · past', def: 'Covered with a shiny grey-white surface, like silver.', ex: 'Frost had silvered the training stones.' },
  disciple: { pos: 'noun', def: 'A follower or student of a teacher, school or sect.', ex: 'the sect’s brightest disciple' },
  woodshed: { pos: 'noun', def: 'A small outbuilding used for storing firewood.', ex: 'the woodshed they had given him' },
  grinding: { pos: 'verb', def: 'Rubbing or pressing hard against something with a harsh noise.', ex: 'grinding ice against the old mill wheel' },
  sect: { pos: 'noun', def: 'In wuxia stories, a martial school with its own rules, elders and techniques.', ex: 'The bell of the Cinder Sect rang' }
};

export const DICT_SUGGEST = ['silvered', 'disciple', 'woodshed', 'grinding'];

export const TRANSLATE_STYLES: TranslateStyle[] = ['Literal', 'Natural', 'Literary', 'Casual'];
