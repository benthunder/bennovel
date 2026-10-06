-- Sample catalog for local development, generated from src/data/mock.ts.
-- Runs on `supabase db reset` (see [db.seed] in config.toml).
-- Every novel gets up to 20 released chapters that reuse the same sample text,
-- like the mock data does. English is the original; Vietnamese and Spanish are human translations.

begin;

insert into public.languages (code, name, native_name, sort_order) values
  ('en', 'English', 'English', 0),
  ('vi', 'Vietnamese', 'Tiếng Việt', 1),
  ('es', 'Spanish', 'Español', 2);

insert into public.categories (slug, name, sort_order) values
  ('wuxia', 'Wuxia', 0),
  ('fantasy', 'Fantasy', 1),
  ('romance', 'Romance', 2),
  ('mystery', 'Mystery', 3),
  ('sci-fi', 'Sci-Fi', 4),
  ('slice-of-life', 'Slice of Life', 5);

insert into public.authors (slug, name) values
  ('tran-linh', 'Tran Linh'),
  ('mira-okafor', 'Mira Okafor'),
  ('hana-seo', 'Hana Seo'),
  ('elias-moreau', 'Elias Moreau'),
  ('jun-takeda', 'Jun Takeda');

insert into public.novels (id, slug, author_id, category_slug, original_lang, status, published_year, cover_palette, rating_avg, rating_count, read_count, is_published)
overriding system value values
  (1, 'ashvale-chronicles', (select id from public.authors where slug = 'tran-linh'), 'wuxia', 'en', 'ongoing', 2023, '{"bg":"var(--color-accent-300)","fg":"var(--color-accent-900)","deco":"var(--color-accent-500)"}'::jsonb, 4.8, 600, 2400000, true),
  (2, 'the-salt-orchard', (select id from public.authors where slug = 'mira-okafor'), 'fantasy', 'en', 'completed', 2021, '{"bg":"var(--color-accent-2-300)","fg":"var(--color-accent-2-900)","deco":"var(--color-accent-2-500)"}'::jsonb, 4.7, 275, 1100000, true),
  (3, 'lanterns-over-kestrel-bay', (select id from public.authors where slug = 'hana-seo'), 'romance', 'en', 'ongoing', 2025, '{"bg":"var(--color-neutral-800)","fg":"var(--color-neutral-100)","deco":"var(--color-accent-600)"}'::jsonb, 4.6, 245, 980000, true),
  (4, 'the-quiet-engine', (select id from public.authors where slug = 'elias-moreau'), 'sci-fi', 'en', 'completed', 2022, '{"bg":"var(--color-accent-600)","fg":"var(--color-accent-100)","deco":"var(--color-accent-400)"}'::jsonb, 4.5, 160, 640000, true),
  (5, 'nine-doors-of-morrow-street', (select id from public.authors where slug = 'jun-takeda'), 'mystery', 'en', 'ongoing', 2024, '{"bg":"var(--color-accent-2-700)","fg":"var(--color-accent-2-100)","deco":"var(--color-accent-2-500)"}'::jsonb, 4.7, 325, 1300000, true),
  (6, 'moss-and-mortar', (select id from public.authors where slug = 'mira-okafor'), 'slice-of-life', 'en', 'completed', 2024, '{"bg":"var(--color-neutral-300)","fg":"var(--color-neutral-900)","deco":"var(--color-accent-300)"}'::jsonb, 4.4, 80, 320000, true),
  (7, 'heir-of-the-ninth-peak', (select id from public.authors where slug = 'tran-linh'), 'wuxia', 'en', 'ongoing', 2023, '{"bg":"var(--color-accent-300)","fg":"var(--color-accent-900)","deco":"var(--color-accent-500)"}'::jsonb, 4.6, 400, 1600000, true),
  (8, 'small-hours-bakery', (select id from public.authors where slug = 'hana-seo'), 'slice-of-life', 'en', 'ongoing', 2026, '{"bg":"var(--color-accent-2-300)","fg":"var(--color-accent-2-900)","deco":"var(--color-accent-2-500)"}'::jsonb, 4.8, 103, 410000, true),
  (9, 'starfall-cartography', (select id from public.authors where slug = 'elias-moreau'), 'sci-fi', 'en', 'ongoing', 2025, '{"bg":"var(--color-neutral-800)","fg":"var(--color-neutral-100)","deco":"var(--color-accent-600)"}'::jsonb, 4.5, 180, 720000, true),
  (10, 'the-glass-heron', (select id from public.authors where slug = 'mira-okafor'), 'fantasy', 'en', 'completed', 2020, '{"bg":"var(--color-accent-600)","fg":"var(--color-accent-100)","deco":"var(--color-accent-400)"}'::jsonb, 4.6, 220, 880000, true),
  (11, 'red-thread-cold-river', (select id from public.authors where slug = 'tran-linh'), 'wuxia', 'en', 'completed', 2022, '{"bg":"var(--color-accent-2-700)","fg":"var(--color-accent-2-100)","deco":"var(--color-accent-2-500)"}'::jsonb, 4.9, 475, 1900000, true),
  (12, 'the-ledger-of-small-crimes', (select id from public.authors where slug = 'jun-takeda'), 'mystery', 'en', 'ongoing', 2026, '{"bg":"var(--color-neutral-300)","fg":"var(--color-neutral-900)","deco":"var(--color-accent-300)"}'::jsonb, 4.3, 38, 150000, true);

select setval(pg_get_serial_sequence('public.novels', 'id'), (select max(id) from public.novels));

insert into public.novel_translations (novel_id, lang, title, description, source) values
  (1, 'en', 'Ashvale Chronicles', 'Stripped of his sword and his rank, a disgraced disciple sweeps the courtyard of the sect that cast him out — until the old janitor starts teaching him forms no elder remembers.', 'original'),
  (1, 'vi', 'Ashvale Chronicles', 'Bị tước kiếm và địa vị, một đệ tử thất sủng quét sân cho chính tông môn đã ruồng bỏ hắn — cho đến khi lão tạp dịch bắt đầu dạy hắn những chiêu thức không trưởng lão nào còn nhớ.', 'human'),
  (2, 'en', 'The Salt Orchard', 'On an island where trees grow salt instead of fruit, a young harvester discovers the orchard is slowly remembering the sea it came from.', 'original'),
  (2, 'vi', 'The Salt Orchard', 'Trên hòn đảo nơi cây kết muối thay vì trái, một cô gái hái muối phát hiện khu vườn đang dần nhớ lại biển cả đã sinh ra nó.', 'human'),
  (3, 'en', 'Lanterns Over Kestrel Bay', 'Two rival lantern-makers are forced to share one workshop for a single festival season. Neither plans on falling for the other.', 'original'),
  (3, 'vi', 'Lanterns Over Kestrel Bay', 'Hai thợ làm đèn lồng đối thủ buộc phải chung một xưởng trong suốt mùa lễ hội. Không ai định phải lòng người kia.', 'human'),
  (4, 'en', 'The Quiet Engine', 'The last engineer on a drifting generation ship hears the engine start to speak — and it wants to turn around.', 'original'),
  (4, 'vi', 'The Quiet Engine', 'Kỹ sư cuối cùng trên con tàu thế hệ trôi dạt nghe thấy động cơ bắt đầu cất tiếng — và nó muốn quay đầu.', 'human'),
  (5, 'en', 'Nine Doors of Morrow Street', 'A locksmith inherits nine keys and a list of nine doors. Behind each one is a crime the city chose to forget.', 'original'),
  (5, 'vi', 'Nine Doors of Morrow Street', 'Một thợ khóa thừa kế chín chiếc chìa và danh sách chín cánh cửa. Sau mỗi cánh cửa là một tội ác thành phố đã chọn quên đi.', 'human'),
  (6, 'en', 'Moss & Mortar', 'A retired stonemason restores a crumbling village wall one stone, one neighbour and one long lunch at a time.', 'original'),
  (6, 'vi', 'Moss & Mortar', 'Người thợ đá về hưu sửa lại bức tường làng đổ nát — từng viên đá, từng người hàng xóm, từng bữa trưa dài.', 'human'),
  (7, 'en', 'Heir of the Ninth Peak', 'The youngest heir of a fallen mountain clan climbs back to the summit his family lost, one duel at a time.', 'original'),
  (7, 'vi', 'Heir of the Ninth Peak', 'Người thừa kế trẻ nhất của một sơn môn sa sút leo lại đỉnh núi gia tộc đã mất, qua từng trận quyết đấu.', 'human'),
  (8, 'en', 'Small Hours Bakery', 'A bakery that only opens from 2 to 5 a.m. feeds night-shift strangers who slowly become a family.', 'original'),
  (8, 'vi', 'Small Hours Bakery', 'Tiệm bánh chỉ mở từ 2 đến 5 giờ sáng nuôi những người lạ làm ca đêm, rồi dần dần họ thành một gia đình.', 'human'),
  (9, 'en', 'Starfall Cartography', 'A mapmaker charts the craters left by falling stars and finds they spell out coordinates.', 'original'),
  (9, 'vi', 'Starfall Cartography', 'Một người vẽ bản đồ ghi lại các hố do sao rơi để lại và nhận ra chúng tạo thành một tọa độ.', 'human'),
  (10, 'en', 'The Glass Heron', 'A glassblower’s apprentice breathes life into a heron that will grant one wish — to whoever can catch it.', 'original'),
  (10, 'vi', 'The Glass Heron', 'Cậu học việc thổi thủy tinh thổi sự sống vào một con diệc sẽ ban một điều ước — cho bất kỳ ai bắt được nó.', 'human'),
  (11, 'en', 'Red Thread, Cold River', 'A swordswoman and the assassin sent to kill her are bound by a red thread neither can cut.', 'original'),
  (11, 'vi', 'Red Thread, Cold River', 'Một nữ kiếm khách và sát thủ được phái đến giết nàng bị trói buộc bởi sợi chỉ đỏ không ai cắt đứt được.', 'human'),
  (12, 'en', 'The Ledger of Small Crimes', 'A small-town accountant keeps a secret ledger of every petty crime — until one entry turns into murder.', 'original'),
  (12, 'vi', 'The Ledger of Small Crimes', 'Một kế toán thị trấn nhỏ ghi sổ bí mật mọi tội vặt — cho đến khi một dòng ghi chép biến thành án mạng.', 'human');

insert into public.collections (key, name, sort_order) values
  ('trend', 'Trending', 0),
  ('new', 'New releases', 1),
  ('done', 'Completed', 2),
  ('picks', 'Editor''s picks', 3);

insert into public.collection_items (collection_key, novel_id, position) values
  ('trend', 1, 0),
  ('trend', 7, 1),
  ('trend', 9, 2),
  ('trend', 5, 3),
  ('trend', 3, 4),
  ('trend', 10, 5),
  ('new', 8, 0),
  ('new', 12, 1),
  ('new', 3, 2),
  ('new', 9, 3),
  ('new', 6, 4),
  ('new', 5, 5),
  ('done', 2, 0),
  ('done', 11, 1),
  ('done', 10, 2),
  ('done', 4, 3),
  ('done', 6, 4),
  ('picks', 6, 0),
  ('picks', 8, 1),
  ('picks', 2, 2),
  ('picks', 4, 3),
  ('picks', 12, 4);

insert into public.chapters (novel_id, number, published_at) values
  (1, 1, now() - interval '20 days'),
  (1, 2, now() - interval '19 days'),
  (1, 3, now() - interval '18 days'),
  (1, 4, now() - interval '17 days'),
  (1, 5, now() - interval '16 days'),
  (1, 6, now() - interval '15 days'),
  (1, 7, now() - interval '14 days'),
  (1, 8, now() - interval '13 days'),
  (1, 9, now() - interval '12 days'),
  (1, 10, now() - interval '11 days'),
  (1, 11, now() - interval '10 days'),
  (1, 12, now() - interval '9 days'),
  (1, 13, now() - interval '8 days'),
  (1, 14, now() - interval '7 days'),
  (1, 15, now() - interval '6 days'),
  (1, 16, now() - interval '5 days'),
  (1, 17, now() - interval '4 days'),
  (1, 18, now() - interval '3 days'),
  (1, 19, now() - interval '2 days'),
  (1, 20, now() - interval '1 days'),
  (2, 1, now() - interval '20 days'),
  (2, 2, now() - interval '19 days'),
  (2, 3, now() - interval '18 days'),
  (2, 4, now() - interval '17 days'),
  (2, 5, now() - interval '16 days'),
  (2, 6, now() - interval '15 days'),
  (2, 7, now() - interval '14 days'),
  (2, 8, now() - interval '13 days'),
  (2, 9, now() - interval '12 days'),
  (2, 10, now() - interval '11 days'),
  (2, 11, now() - interval '10 days'),
  (2, 12, now() - interval '9 days'),
  (2, 13, now() - interval '8 days'),
  (2, 14, now() - interval '7 days'),
  (2, 15, now() - interval '6 days'),
  (2, 16, now() - interval '5 days'),
  (2, 17, now() - interval '4 days'),
  (2, 18, now() - interval '3 days'),
  (2, 19, now() - interval '2 days'),
  (2, 20, now() - interval '1 days'),
  (3, 1, now() - interval '20 days'),
  (3, 2, now() - interval '19 days'),
  (3, 3, now() - interval '18 days'),
  (3, 4, now() - interval '17 days'),
  (3, 5, now() - interval '16 days'),
  (3, 6, now() - interval '15 days'),
  (3, 7, now() - interval '14 days'),
  (3, 8, now() - interval '13 days'),
  (3, 9, now() - interval '12 days'),
  (3, 10, now() - interval '11 days'),
  (3, 11, now() - interval '10 days'),
  (3, 12, now() - interval '9 days'),
  (3, 13, now() - interval '8 days'),
  (3, 14, now() - interval '7 days'),
  (3, 15, now() - interval '6 days'),
  (3, 16, now() - interval '5 days'),
  (3, 17, now() - interval '4 days'),
  (3, 18, now() - interval '3 days'),
  (3, 19, now() - interval '2 days'),
  (3, 20, now() - interval '1 days'),
  (4, 1, now() - interval '20 days'),
  (4, 2, now() - interval '19 days'),
  (4, 3, now() - interval '18 days'),
  (4, 4, now() - interval '17 days'),
  (4, 5, now() - interval '16 days'),
  (4, 6, now() - interval '15 days'),
  (4, 7, now() - interval '14 days'),
  (4, 8, now() - interval '13 days'),
  (4, 9, now() - interval '12 days'),
  (4, 10, now() - interval '11 days'),
  (4, 11, now() - interval '10 days'),
  (4, 12, now() - interval '9 days'),
  (4, 13, now() - interval '8 days'),
  (4, 14, now() - interval '7 days'),
  (4, 15, now() - interval '6 days'),
  (4, 16, now() - interval '5 days'),
  (4, 17, now() - interval '4 days'),
  (4, 18, now() - interval '3 days'),
  (4, 19, now() - interval '2 days'),
  (4, 20, now() - interval '1 days'),
  (5, 1, now() - interval '20 days'),
  (5, 2, now() - interval '19 days'),
  (5, 3, now() - interval '18 days'),
  (5, 4, now() - interval '17 days'),
  (5, 5, now() - interval '16 days'),
  (5, 6, now() - interval '15 days'),
  (5, 7, now() - interval '14 days'),
  (5, 8, now() - interval '13 days'),
  (5, 9, now() - interval '12 days'),
  (5, 10, now() - interval '11 days'),
  (5, 11, now() - interval '10 days'),
  (5, 12, now() - interval '9 days'),
  (5, 13, now() - interval '8 days'),
  (5, 14, now() - interval '7 days'),
  (5, 15, now() - interval '6 days'),
  (5, 16, now() - interval '5 days'),
  (5, 17, now() - interval '4 days'),
  (5, 18, now() - interval '3 days'),
  (5, 19, now() - interval '2 days'),
  (5, 20, now() - interval '1 days'),
  (6, 1, now() - interval '20 days'),
  (6, 2, now() - interval '19 days'),
  (6, 3, now() - interval '18 days'),
  (6, 4, now() - interval '17 days'),
  (6, 5, now() - interval '16 days'),
  (6, 6, now() - interval '15 days'),
  (6, 7, now() - interval '14 days'),
  (6, 8, now() - interval '13 days'),
  (6, 9, now() - interval '12 days'),
  (6, 10, now() - interval '11 days'),
  (6, 11, now() - interval '10 days'),
  (6, 12, now() - interval '9 days'),
  (6, 13, now() - interval '8 days'),
  (6, 14, now() - interval '7 days'),
  (6, 15, now() - interval '6 days'),
  (6, 16, now() - interval '5 days'),
  (6, 17, now() - interval '4 days'),
  (6, 18, now() - interval '3 days'),
  (6, 19, now() - interval '2 days'),
  (6, 20, now() - interval '1 days'),
  (7, 1, now() - interval '20 days'),
  (7, 2, now() - interval '19 days'),
  (7, 3, now() - interval '18 days'),
  (7, 4, now() - interval '17 days'),
  (7, 5, now() - interval '16 days'),
  (7, 6, now() - interval '15 days'),
  (7, 7, now() - interval '14 days'),
  (7, 8, now() - interval '13 days'),
  (7, 9, now() - interval '12 days'),
  (7, 10, now() - interval '11 days'),
  (7, 11, now() - interval '10 days'),
  (7, 12, now() - interval '9 days'),
  (7, 13, now() - interval '8 days'),
  (7, 14, now() - interval '7 days'),
  (7, 15, now() - interval '6 days'),
  (7, 16, now() - interval '5 days'),
  (7, 17, now() - interval '4 days'),
  (7, 18, now() - interval '3 days'),
  (7, 19, now() - interval '2 days'),
  (7, 20, now() - interval '1 days'),
  (8, 1, now() - interval '20 days'),
  (8, 2, now() - interval '19 days'),
  (8, 3, now() - interval '18 days'),
  (8, 4, now() - interval '17 days'),
  (8, 5, now() - interval '16 days'),
  (8, 6, now() - interval '15 days'),
  (8, 7, now() - interval '14 days'),
  (8, 8, now() - interval '13 days'),
  (8, 9, now() - interval '12 days'),
  (8, 10, now() - interval '11 days'),
  (8, 11, now() - interval '10 days'),
  (8, 12, now() - interval '9 days'),
  (8, 13, now() - interval '8 days'),
  (8, 14, now() - interval '7 days'),
  (8, 15, now() - interval '6 days'),
  (8, 16, now() - interval '5 days'),
  (8, 17, now() - interval '4 days'),
  (8, 18, now() - interval '3 days'),
  (8, 19, now() - interval '2 days'),
  (8, 20, now() - interval '1 days'),
  (9, 1, now() - interval '20 days'),
  (9, 2, now() - interval '19 days'),
  (9, 3, now() - interval '18 days'),
  (9, 4, now() - interval '17 days'),
  (9, 5, now() - interval '16 days'),
  (9, 6, now() - interval '15 days'),
  (9, 7, now() - interval '14 days'),
  (9, 8, now() - interval '13 days'),
  (9, 9, now() - interval '12 days'),
  (9, 10, now() - interval '11 days'),
  (9, 11, now() - interval '10 days'),
  (9, 12, now() - interval '9 days'),
  (9, 13, now() - interval '8 days'),
  (9, 14, now() - interval '7 days'),
  (9, 15, now() - interval '6 days'),
  (9, 16, now() - interval '5 days'),
  (9, 17, now() - interval '4 days'),
  (9, 18, now() - interval '3 days'),
  (9, 19, now() - interval '2 days'),
  (9, 20, now() - interval '1 days'),
  (10, 1, now() - interval '20 days'),
  (10, 2, now() - interval '19 days'),
  (10, 3, now() - interval '18 days'),
  (10, 4, now() - interval '17 days'),
  (10, 5, now() - interval '16 days'),
  (10, 6, now() - interval '15 days'),
  (10, 7, now() - interval '14 days'),
  (10, 8, now() - interval '13 days'),
  (10, 9, now() - interval '12 days'),
  (10, 10, now() - interval '11 days'),
  (10, 11, now() - interval '10 days'),
  (10, 12, now() - interval '9 days'),
  (10, 13, now() - interval '8 days'),
  (10, 14, now() - interval '7 days'),
  (10, 15, now() - interval '6 days'),
  (10, 16, now() - interval '5 days'),
  (10, 17, now() - interval '4 days'),
  (10, 18, now() - interval '3 days'),
  (10, 19, now() - interval '2 days'),
  (10, 20, now() - interval '1 days'),
  (11, 1, now() - interval '20 days'),
  (11, 2, now() - interval '19 days'),
  (11, 3, now() - interval '18 days'),
  (11, 4, now() - interval '17 days'),
  (11, 5, now() - interval '16 days'),
  (11, 6, now() - interval '15 days'),
  (11, 7, now() - interval '14 days'),
  (11, 8, now() - interval '13 days'),
  (11, 9, now() - interval '12 days'),
  (11, 10, now() - interval '11 days'),
  (11, 11, now() - interval '10 days'),
  (11, 12, now() - interval '9 days'),
  (11, 13, now() - interval '8 days'),
  (11, 14, now() - interval '7 days'),
  (11, 15, now() - interval '6 days'),
  (11, 16, now() - interval '5 days'),
  (11, 17, now() - interval '4 days'),
  (11, 18, now() - interval '3 days'),
  (11, 19, now() - interval '2 days'),
  (11, 20, now() - interval '1 days'),
  (12, 1, now() - interval '20 days'),
  (12, 2, now() - interval '19 days'),
  (12, 3, now() - interval '18 days'),
  (12, 4, now() - interval '17 days'),
  (12, 5, now() - interval '16 days'),
  (12, 6, now() - interval '15 days'),
  (12, 7, now() - interval '14 days'),
  (12, 8, now() - interval '13 days'),
  (12, 9, now() - interval '12 days'),
  (12, 10, now() - interval '11 days'),
  (12, 11, now() - interval '10 days'),
  (12, 12, now() - interval '9 days'),
  (12, 13, now() - interval '8 days'),
  (12, 14, now() - interval '7 days'),
  (12, 15, now() - interval '6 days'),
  (12, 16, now() - interval '5 days'),
  (12, 17, now() - interval '4 days'),
  (12, 18, now() - interval '3 days'),
  (12, 19, now() - interval '2 days'),
  (12, 20, now() - interval '1 days');

insert into public.chapter_translations (chapter_id, lang, source, state, title, content)
select c.id, 'en', 'original', 'published', t.title, 'The bell of the [[sect]] rang three times before dawn, and [[wen]] was already awake, counting the cracks in the ceiling of the woodshed they had given him instead of a room.

Six months ago he had been the sect’s brightest disciple. Now his sword hung on the wall of the elders’ hall, wrapped in white cloth like something that had died.

He dressed in the dark, tied back his sleeves and stepped into the yard. Frost had silvered the training stones. Somewhere beyond the wall, the river that gave [[ash]] its name was grinding ice against the old mill wheel.

“You’re early,” said a voice from the gate. Old Master Qiao leaned on his broom as if it were the only thing holding up the morning. “Or you never slept.”

“Both,” said [[wen]].

The old man laughed, a dry sound like paper tearing. “Then sweep. A man who cannot hold a sword can still hold a broom — and a broom, boy, has taught more than one fool how to stand.”

[[wen]] took the broom. It was heavier than it looked.'
  from public.chapters c
  join (values
    (1, 'The Bell Before Dawn'),
    (2, 'A Broom Heavier Than Steel'),
    (3, 'Frost on the Training Stones'),
    (4, 'The Elders’ Hall'),
    (5, 'What the River Remembers'),
    (6, 'Ash in the Rice Bowl'),
    (7, 'A Sword in White Cloth'),
    (8, 'The Mill Wheel Turns'),
    (9, 'Old Master Qiao'),
    (10, 'Lessons in Standing'),
    (11, 'Smoke Over the Eastern Ridge'),
    (12, 'The First Cut'),
    (13, 'Night Market'),
    (14, 'A Debt of Salt'),
    (15, 'Paper Lanterns'),
    (16, 'The Second Bell'),
    (17, 'Snow Without Sound'),
    (18, 'Iron and Patience'),
    (19, 'Borrowed Names'),
    (20, 'Where the Road Bends')
  ) as t(number, title) on t.number = c.number;

insert into public.chapter_translations (chapter_id, lang, source, state, title, content)
select c.id, 'vi', 'human', 'published', t.title, 'Chuông của [[sect]] vang ba hồi trước lúc rạng đông, và [[wen]] đã thức từ lâu, đếm những vết nứt trên trần căn kho củi mà người ta cho hắn ở thay cho một gian phòng.

Sáu tháng trước, hắn là đệ tử sáng giá nhất của tông môn. Giờ đây thanh kiếm của hắn treo trên vách đại sảnh trưởng lão, bọc trong vải trắng như một thứ đã chết.

Hắn mặc áo trong bóng tối, buộc gọn tay áo rồi bước ra sân. Sương giá đã phủ bạc những phiến đá luyện công. Đâu đó sau bức tường, con sông đã đặt tên cho [[ash]] đang nghiến băng vào chiếc guồng nước cũ.

“Ngươi dậy sớm,” một giọng nói vang lên từ cổng. Lão Kiều chống chổi như thể đó là thứ duy nhất giữ cho buổi sáng đứng vững. “Hay là ngươi chẳng hề ngủ.”

“Cả hai,” [[wen]] đáp.

Lão già bật cười, tiếng cười khô khốc như giấy rách. “Vậy thì quét đi. Kẻ không cầm nổi kiếm vẫn cầm được chổi — mà cái chổi, nhóc à, đã dạy không ít kẻ ngốc cách đứng vững.”

[[wen]] cầm lấy cây chổi. Nó nặng hơn vẻ ngoài.'
  from public.chapters c
  join (values
    (1, 'The Bell Before Dawn'),
    (2, 'A Broom Heavier Than Steel'),
    (3, 'Frost on the Training Stones'),
    (4, 'The Elders’ Hall'),
    (5, 'What the River Remembers'),
    (6, 'Ash in the Rice Bowl'),
    (7, 'A Sword in White Cloth'),
    (8, 'The Mill Wheel Turns'),
    (9, 'Old Master Qiao'),
    (10, 'Lessons in Standing'),
    (11, 'Smoke Over the Eastern Ridge'),
    (12, 'The First Cut'),
    (13, 'Night Market'),
    (14, 'A Debt of Salt'),
    (15, 'Paper Lanterns'),
    (16, 'The Second Bell'),
    (17, 'Snow Without Sound'),
    (18, 'Iron and Patience'),
    (19, 'Borrowed Names'),
    (20, 'Where the Road Bends')
  ) as t(number, title) on t.number = c.number;

insert into public.chapter_translations (chapter_id, lang, source, state, title, content)
select c.id, 'es', 'human', 'published', t.title, 'La campana de la [[sect]] sonó tres veces antes del alba, y [[wen]] ya estaba despierto, contando las grietas del techo de la leñera que le habían dado en lugar de una habitación.

Hace seis meses había sido el discípulo más brillante de la secta. Ahora su espada colgaba en la pared del salón de los ancianos, envuelta en tela blanca como algo que hubiera muerto.

Se vistió a oscuras, se ató las mangas y salió al patio. La escarcha había plateado las piedras de entrenamiento. Más allá del muro, el río que daba nombre a [[ash]] arrastraba hielo contra la vieja rueda del molino.

—Llegas temprano —dijo una voz desde la puerta. El viejo maestro Qiao se apoyaba en su escoba como si fuera lo único que sostenía la mañana—. O no has dormido.

—Las dos cosas —dijo [[wen]].

El viejo rió, un sonido seco como papel que se rasga. —Entonces barre. Quien no puede sostener una espada aún puede sostener una escoba, y una escoba, muchacho, ha enseñado a más de un tonto a mantenerse en pie.

[[wen]] tomó la escoba. Pesaba más de lo que parecía.'
  from public.chapters c
  join (values
    (1, 'The Bell Before Dawn'),
    (2, 'A Broom Heavier Than Steel'),
    (3, 'Frost on the Training Stones'),
    (4, 'The Elders’ Hall'),
    (5, 'What the River Remembers'),
    (6, 'Ash in the Rice Bowl'),
    (7, 'A Sword in White Cloth'),
    (8, 'The Mill Wheel Turns'),
    (9, 'Old Master Qiao'),
    (10, 'Lessons in Standing'),
    (11, 'Smoke Over the Eastern Ridge'),
    (12, 'The First Cut'),
    (13, 'Night Market'),
    (14, 'A Debt of Salt'),
    (15, 'Paper Lanterns'),
    (16, 'The Second Bell'),
    (17, 'Snow Without Sound'),
    (18, 'Iron and Patience'),
    (19, 'Borrowed Names'),
    (20, 'Where the Road Bends')
  ) as t(number, title) on t.number = c.number;

insert into public.glossary_entries (novel_id, term_key, lang, value) values
  (1, 'sect', 'en', 'Cinder Sect'),
  (1, 'sect', 'vi', 'Hỏa Tẫn Tông'),
  (1, 'sect', 'es', 'Secta Ceniza'),
  (1, 'wen', 'en', 'Wen Yu'),
  (1, 'wen', 'vi', 'Văn Vũ'),
  (1, 'wen', 'es', 'Wen Yu'),
  (1, 'ash', 'en', 'Ashvale'),
  (1, 'ash', 'vi', 'Tro Cốc'),
  (1, 'ash', 'es', 'Valceniza');

commit;

