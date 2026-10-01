import type { Category, Collection, ContentLang, DictEntry, GlossaryTerm, Novel, NovelStatus, TranslateStyle } from './types';

const PAL: Novel['cover'][] = [
  { bg: 'var(--color-accent-300)', fg: 'var(--color-accent-900)', deco: 'var(--color-accent-500)' },
  { bg: 'var(--color-accent-2-300)', fg: 'var(--color-accent-2-900)', deco: 'var(--color-accent-2-500)' },
  { bg: 'var(--color-neutral-800)', fg: 'var(--color-neutral-100)', deco: 'var(--color-accent-600)' },
  { bg: 'var(--color-accent-600)', fg: 'var(--color-accent-100)', deco: 'var(--color-accent-400)' },
  { bg: 'var(--color-accent-2-700)', fg: 'var(--color-accent-2-100)', deco: 'var(--color-accent-2-500)' },
  { bg: 'var(--color-neutral-300)', fg: 'var(--color-neutral-900)', deco: 'var(--color-accent-300)' }
];

type Row = [number, string, string, Category, NovelStatus, number, number, number, number, string, string];

const ROWS: Row[] = [
  [1, 'Ashvale Chronicles', 'Tran Linh', 'Wuxia', 'Ongoing', 412, 4.8, 2400, 2023,
    'Stripped of his sword and his rank, a disgraced disciple sweeps the courtyard of the sect that cast him out — until the old janitor starts teaching him forms no elder remembers.',
    'Bị tước kiếm và địa vị, một đệ tử thất sủng quét sân cho chính tông môn đã ruồng bỏ hắn — cho đến khi lão tạp dịch bắt đầu dạy hắn những chiêu thức không trưởng lão nào còn nhớ.'],
  [2, 'The Salt Orchard', 'Mira Okafor', 'Fantasy', 'Completed', 128, 4.7, 1100, 2021,
    'On an island where trees grow salt instead of fruit, a young harvester discovers the orchard is slowly remembering the sea it came from.',
    'Trên hòn đảo nơi cây kết muối thay vì trái, một cô gái hái muối phát hiện khu vườn đang dần nhớ lại biển cả đã sinh ra nó.'],
  [3, 'Lanterns Over Kestrel Bay', 'Hana Seo', 'Romance', 'Ongoing', 86, 4.6, 980, 2025,
    'Two rival lantern-makers are forced to share one workshop for a single festival season. Neither plans on falling for the other.',
    'Hai thợ làm đèn lồng đối thủ buộc phải chung một xưởng trong suốt mùa lễ hội. Không ai định phải lòng người kia.'],
  [4, 'The Quiet Engine', 'Elias Moreau', 'Sci-Fi', 'Completed', 64, 4.5, 640, 2022,
    'The last engineer on a drifting generation ship hears the engine start to speak — and it wants to turn around.',
    'Kỹ sư cuối cùng trên con tàu thế hệ trôi dạt nghe thấy động cơ bắt đầu cất tiếng — và nó muốn quay đầu.'],
  [5, 'Nine Doors of Morrow Street', 'Jun Takeda', 'Mystery', 'Ongoing', 150, 4.7, 1300, 2024,
    'A locksmith inherits nine keys and a list of nine doors. Behind each one is a crime the city chose to forget.',
    'Một thợ khóa thừa kế chín chiếc chìa và danh sách chín cánh cửa. Sau mỗi cánh cửa là một tội ác thành phố đã chọn quên đi.'],
  [6, 'Moss & Mortar', 'Mira Okafor', 'Slice of Life', 'Completed', 42, 4.4, 320, 2024,
    'A retired stonemason restores a crumbling village wall one stone, one neighbour and one long lunch at a time.',
    'Người thợ đá về hưu sửa lại bức tường làng đổ nát — từng viên đá, từng người hàng xóm, từng bữa trưa dài.'],
  [7, 'Heir of the Ninth Peak', 'Tran Linh', 'Wuxia', 'Ongoing', 238, 4.6, 1600, 2023,
    'The youngest heir of a fallen mountain clan climbs back to the summit his family lost, one duel at a time.',
    'Người thừa kế trẻ nhất của một sơn môn sa sút leo lại đỉnh núi gia tộc đã mất, qua từng trận quyết đấu.'],
  [8, 'Small Hours Bakery', 'Hana Seo', 'Slice of Life', 'Ongoing', 57, 4.8, 410, 2026,
    'A bakery that only opens from 2 to 5 a.m. feeds night-shift strangers who slowly become a family.',
    'Tiệm bánh chỉ mở từ 2 đến 5 giờ sáng nuôi những người lạ làm ca đêm, rồi dần dần họ thành một gia đình.'],
  [9, 'Starfall Cartography', 'Elias Moreau', 'Sci-Fi', 'Ongoing', 95, 4.5, 720, 2025,
    'A mapmaker charts the craters left by falling stars and finds they spell out coordinates.',
    'Một người vẽ bản đồ ghi lại các hố do sao rơi để lại và nhận ra chúng tạo thành một tọa độ.'],
  [10, 'The Glass Heron', 'Mira Okafor', 'Fantasy', 'Completed', 110, 4.6, 880, 2020,
    'A glassblower’s apprentice breathes life into a heron that will grant one wish — to whoever can catch it.',
    'Cậu học việc thổi thủy tinh thổi sự sống vào một con diệc sẽ ban một điều ước — cho bất kỳ ai bắt được nó.'],
  [11, 'Red Thread, Cold River', 'Tran Linh', 'Wuxia', 'Completed', 176, 4.9, 1900, 2022,
    'A swordswoman and the assassin sent to kill her are bound by a red thread neither can cut.',
    'Một nữ kiếm khách và sát thủ được phái đến giết nàng bị trói buộc bởi sợi chỉ đỏ không ai cắt đứt được.'],
  [12, 'The Ledger of Small Crimes', 'Jun Takeda', 'Mystery', 'Ongoing', 33, 4.3, 150, 2026,
    'A small-town accountant keeps a secret ledger of every petty crime — until one entry turns into murder.',
    'Một kế toán thị trấn nhỏ ghi sổ bí mật mọi tội vặt — cho đến khi một dòng ghi chép biến thành án mạng.']
];

export const NOVELS: Novel[] = ROWS.map(([id, title, author, cat, status, chapters, rating, reads, year, en, vi], i) => ({
  id, title, author, cat, status, chapters, rating, reads, year, desc: { en, vi }, cover: PAL[i % PAL.length]
}));

export const CATEGORIES: Category[] = ['Wuxia', 'Fantasy', 'Romance', 'Mystery', 'Sci-Fi', 'Slice of Life'];

export const POPULAR_IDS = [1, 11, 7, 5, 2];

export const COLLECTIONS: Collection[] = [
  { key: 'trend', ids: [1, 7, 9, 5, 3, 10] },
  { key: 'new', ids: [8, 12, 3, 9, 6, 5] },
  { key: 'done', ids: [2, 11, 10, 4, 6] },
  { key: 'picks', ids: [6, 8, 2, 4, 12] }
];

export const CONTENT_LANGS: { code: ContentLang; name: string; native: string }[] = [
  { code: 'en', name: 'English', native: 'English' },
  { code: 'vi', name: 'Vietnamese', native: 'Tiếng Việt' },
  { code: 'es', name: 'Spanish', native: 'Español' }
];

export const GLOSSARY: Record<string, GlossaryTerm> = {
  sect: { en: 'Cinder Sect', vi: 'Hỏa Tẫn Tông', es: 'Secta Ceniza' },
  wen: { en: 'Wen Yu', vi: 'Văn Vũ', es: 'Wen Yu' },
  ash: { en: 'Ashvale', vi: 'Tro Cốc', es: 'Valceniza' }
};

/** Sample chapter text. `[[key]]` marks a glossary term. Every novel shows this chapter for now. */
export const SAMPLE_TEXT: Record<ContentLang, string[]> = {
  en: [
    'The bell of the [[sect]] rang three times before dawn, and [[wen]] was already awake, counting the cracks in the ceiling of the woodshed they had given him instead of a room.',
    'Six months ago he had been the sect’s brightest disciple. Now his sword hung on the wall of the elders’ hall, wrapped in white cloth like something that had died.',
    'He dressed in the dark, tied back his sleeves and stepped into the yard. Frost had silvered the training stones. Somewhere beyond the wall, the river that gave [[ash]] its name was grinding ice against the old mill wheel.',
    '“You’re early,” said a voice from the gate. Old Master Qiao leaned on his broom as if it were the only thing holding up the morning. “Or you never slept.”',
    '“Both,” said [[wen]].',
    'The old man laughed, a dry sound like paper tearing. “Then sweep. A man who cannot hold a sword can still hold a broom — and a broom, boy, has taught more than one fool how to stand.”',
    '[[wen]] took the broom. It was heavier than it looked.'
  ],
  vi: [
    'Chuông của [[sect]] vang ba hồi trước lúc rạng đông, và [[wen]] đã thức từ lâu, đếm những vết nứt trên trần căn kho củi mà người ta cho hắn ở thay cho một gian phòng.',
    'Sáu tháng trước, hắn là đệ tử sáng giá nhất của tông môn. Giờ đây thanh kiếm của hắn treo trên vách đại sảnh trưởng lão, bọc trong vải trắng như một thứ đã chết.',
    'Hắn mặc áo trong bóng tối, buộc gọn tay áo rồi bước ra sân. Sương giá đã phủ bạc những phiến đá luyện công. Đâu đó sau bức tường, con sông đã đặt tên cho [[ash]] đang nghiến băng vào chiếc guồng nước cũ.',
    '“Ngươi dậy sớm,” một giọng nói vang lên từ cổng. Lão Kiều chống chổi như thể đó là thứ duy nhất giữ cho buổi sáng đứng vững. “Hay là ngươi chẳng hề ngủ.”',
    '“Cả hai,” [[wen]] đáp.',
    'Lão già bật cười, tiếng cười khô khốc như giấy rách. “Vậy thì quét đi. Kẻ không cầm nổi kiếm vẫn cầm được chổi — mà cái chổi, nhóc à, đã dạy không ít kẻ ngốc cách đứng vững.”',
    '[[wen]] cầm lấy cây chổi. Nó nặng hơn vẻ ngoài.'
  ],
  es: [
    'La campana de la [[sect]] sonó tres veces antes del alba, y [[wen]] ya estaba despierto, contando las grietas del techo de la leñera que le habían dado en lugar de una habitación.',
    'Hace seis meses había sido el discípulo más brillante de la secta. Ahora su espada colgaba en la pared del salón de los ancianos, envuelta en tela blanca como algo que hubiera muerto.',
    'Se vistió a oscuras, se ató las mangas y salió al patio. La escarcha había plateado las piedras de entrenamiento. Más allá del muro, el río que daba nombre a [[ash]] arrastraba hielo contra la vieja rueda del molino.',
    '—Llegas temprano —dijo una voz desde la puerta. El viejo maestro Qiao se apoyaba en su escoba como si fuera lo único que sostenía la mañana—. O no has dormido.',
    '—Las dos cosas —dijo [[wen]].',
    'El viejo rió, un sonido seco como papel que se rasga. —Entonces barre. Quien no puede sostener una espada aún puede sostener una escoba, y una escoba, muchacho, ha enseñado a más de un tonto a mantenerse en pie.',
    '[[wen]] tomó la escoba. Pesaba más de lo que parecía.'
  ]
};

export const CHAPTER_TITLES = ['The Bell Before Dawn', 'A Broom Heavier Than Steel', 'Frost on the Training Stones', 'The Elders’ Hall', 'What the River Remembers', 'Ash in the Rice Bowl', 'A Sword in White Cloth', 'The Mill Wheel Turns', 'Old Master Qiao', 'Lessons in Standing', 'Smoke Over the Eastern Ridge', 'The First Cut', 'Night Market', 'A Debt of Salt', 'Paper Lanterns', 'The Second Bell', 'Snow Without Sound', 'Iron and Patience', 'Borrowed Names', 'Where the Road Bends'];

export const DICTIONARY: Record<string, DictEntry> = {
  silvered: { pos: 'verb · past', def: 'Covered with a shiny grey-white surface, like silver.', ex: 'Frost had silvered the training stones.' },
  disciple: { pos: 'noun', def: 'A follower or student of a teacher, school or sect.', ex: 'the sect’s brightest disciple' },
  woodshed: { pos: 'noun', def: 'A small outbuilding used for storing firewood.', ex: 'the woodshed they had given him' },
  grinding: { pos: 'verb', def: 'Rubbing or pressing hard against something with a harsh noise.', ex: 'grinding ice against the old mill wheel' },
  sect: { pos: 'noun', def: 'In wuxia stories, a martial school with its own rules, elders and techniques.', ex: 'The bell of the Cinder Sect rang' }
};

export const DICT_SUGGEST = ['silvered', 'disciple', 'woodshed', 'grinding'];

export const TRANSLATE_STYLES: TranslateStyle[] = ['Literal', 'Natural', 'Literary', 'Casual'];
