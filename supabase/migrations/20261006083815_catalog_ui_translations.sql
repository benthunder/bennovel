-- Category and collection names in the app's interface language.
-- The interface ships in English and Vietnamese only (same set as profiles.ui_lang),
-- so these tables are limited to 'en' and 'vi'. Reading languages for novel and
-- chapter text stay open-ended in novel_translations / chapter_translations.
-- categories.name and collections.name remain the internal (admin) names.

create table public.category_translations (
  category_slug text not null references public.categories (slug) on update cascade on delete cascade,
  lang          text not null check (lang in ('en', 'vi')),
  name          text not null,
  primary key (category_slug, lang)
);

create table public.collection_translations (
  collection_key text not null references public.collections (key) on update cascade on delete cascade,
  lang           text not null check (lang in ('en', 'vi')),
  -- Small line above the shelf title on Home ("This week").
  kicker         text not null default '',
  title          text not null,
  primary key (collection_key, lang)
);

alter table public.category_translations   enable row level security;
alter table public.collection_translations enable row level security;

create policy "category translations are public" on public.category_translations
  for select to anon, authenticated using (true);

create policy "collection translations are public" on public.collection_translations
  for select to anon, authenticated using (true);

-- Names for the categories and collections that already exist. The join skips
-- rows whose parent is missing (a fresh `db reset` runs this before seed.sql).
insert into public.category_translations (category_slug, lang, name)
select v.slug, v.lang, v.name
from (values
  ('wuxia', 'en', 'Wuxia'),
  ('wuxia', 'vi', 'Kiếm hiệp'),
  ('fantasy', 'en', 'Fantasy'),
  ('fantasy', 'vi', 'Kỳ ảo'),
  ('romance', 'en', 'Romance'),
  ('romance', 'vi', 'Lãng mạn'),
  ('mystery', 'en', 'Mystery'),
  ('mystery', 'vi', 'Trinh thám'),
  ('sci-fi', 'en', 'Sci-Fi'),
  ('sci-fi', 'vi', 'Khoa học viễn tưởng'),
  ('slice-of-life', 'en', 'Slice of Life'),
  ('slice-of-life', 'vi', 'Đời thường')
) as v (slug, lang, name)
join public.categories c on c.slug = v.slug
on conflict do nothing;

insert into public.collection_translations (collection_key, lang, kicker, title)
select v.key, v.lang, v.kicker, v.title
from (values
  ('trend', 'en', 'This week', 'Trending now'),
  ('trend', 'vi', 'Tuần này', 'Đang thịnh hành'),
  ('new', 'en', 'Fresh ink', 'New releases'),
  ('new', 'vi', 'Mới ra lò', 'Truyện mới'),
  ('done', 'en', 'Binge-ready', 'Completed sagas'),
  ('done', 'vi', 'Đọc một lèo', 'Truyện đã hoàn thành'),
  ('picks', 'en', 'From the editors', 'Quiet favourites'),
  ('picks', 'vi', 'Biên tập chọn', 'Những cuốn dịu dàng')
) as v (key, lang, kicker, title)
join public.collections c on c.key = v.key
on conflict do nothing;
