-- Per-user data: profile and settings, favourites, reading history.
-- Each user sees and changes only their own rows.

-- ---------------------------------------------------------------------------
-- Profiles (1:1 with auth.users, created automatically on sign-up)
-- ---------------------------------------------------------------------------

create table public.profiles (
  id                   uuid primary key references auth.users (id) on delete cascade,
  display_name         text,
  avatar_url           text,
  -- Interface language (the app ships 'en' and 'vi').
  ui_lang              text not null default 'en' check (ui_lang in ('en', 'vi')),
  -- Reading language picked in the language dialog; null = ask on first chapter.
  default_content_lang text references public.languages (code),
  -- Reader settings (font size, theme, line height, ...). Shape is owned by the app.
  reader_prefs         jsonb not null default '{}'::jsonb check (jsonb_typeof(reader_prefs) = 'object'),
  created_at           timestamptz not null default now(),
  updated_at           timestamptz not null default now()
);

create trigger profiles_set_updated_at
before update on public.profiles
for each row execute function public.set_updated_at();

-- Runs as the table owner so it can write to public.profiles from the auth schema trigger.
create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer
set search_path = ''
as $$
begin
  insert into public.profiles (id, display_name, avatar_url)
  values (
    new.id,
    coalesce(new.raw_user_meta_data ->> 'full_name', new.raw_user_meta_data ->> 'name', split_part(new.email, '@', 1)),
    new.raw_user_meta_data ->> 'avatar_url'
  );
  return new;
end;
$$;

revoke execute on function public.handle_new_user() from public, anon, authenticated;

create trigger on_auth_user_created
after insert on auth.users
for each row execute function public.handle_new_user();

-- ---------------------------------------------------------------------------
-- Favourites (the heart button on Detail, the grid in Library)
-- ---------------------------------------------------------------------------

create table public.favorites (
  user_id    uuid not null default auth.uid() references auth.users (id) on delete cascade,
  novel_id   bigint not null references public.novels (id) on delete cascade,
  created_at timestamptz not null default now(),
  primary key (user_id, novel_id)
);

create index favorites_novel_id_idx on public.favorites (novel_id);

-- ---------------------------------------------------------------------------
-- Reading history: one row per (user, novel), holding where they stopped.
-- Library's history list and the Continue button read from this.
-- ---------------------------------------------------------------------------

create table public.reading_history (
  user_id         uuid not null default auth.uid() references auth.users (id) on delete cascade,
  novel_id        bigint not null references public.novels (id) on delete cascade,
  chapter_id      bigint not null references public.chapters (id) on delete cascade,
  chapter_number  integer not null check (chapter_number > 0),
  lang            text not null references public.languages (code),
  -- Scroll position inside the chapter, 0..1.
  progress        real not null default 0 check (progress between 0 and 1),
  last_read_at    timestamptz not null default now(),
  primary key (user_id, novel_id)
);

create index reading_history_user_recent_idx on public.reading_history (user_id, last_read_at desc);
create index reading_history_chapter_id_idx on public.reading_history (chapter_id);
create index reading_history_novel_id_idx on public.reading_history (novel_id);

-- Keep chapter_number and novel_id consistent with chapter_id, and bump last_read_at.
create or replace function public.reading_history_fill()
returns trigger
language plpgsql
set search_path = ''
as $$
declare
  ch record;
begin
  select c.novel_id, c.number into ch from public.chapters c where c.id = new.chapter_id;
  if ch.novel_id is distinct from new.novel_id then
    raise exception 'chapter % does not belong to novel %', new.chapter_id, new.novel_id
      using errcode = '23514';
  end if;
  new.chapter_number := ch.number;
  if tg_op = 'UPDATE' then
    new.last_read_at := now();
  end if;
  return new;
end;
$$;

create trigger reading_history_fill
before insert or update on public.reading_history
for each row execute function public.reading_history_fill();

-- First time a user opens a novel counts as one read. Readers cannot update
-- novels themselves, so this runs as the table owner.
create or replace function public.count_novel_read()
returns trigger
language plpgsql
security definer
set search_path = ''
as $$
begin
  update public.novels set read_count = read_count + 1 where id = new.novel_id;
  return null;
end;
$$;

revoke execute on function public.count_novel_read() from public, anon, authenticated;

create trigger reading_history_count_read
after insert on public.reading_history
for each row execute function public.count_novel_read();

-- ---------------------------------------------------------------------------
-- Row level security
-- ---------------------------------------------------------------------------

alter table public.profiles        enable row level security;
alter table public.favorites       enable row level security;
alter table public.reading_history enable row level security;

-- Profiles are created by the trigger and removed with the auth user,
-- so users only get select and update.
create policy "users read own profile" on public.profiles
  for select to authenticated using (id = (select auth.uid()));

create policy "users update own profile" on public.profiles
  for update to authenticated
  using (id = (select auth.uid()))
  with check (id = (select auth.uid()));

create policy "users read own favorites" on public.favorites
  for select to authenticated using (user_id = (select auth.uid()));

create policy "users add own favorites" on public.favorites
  for insert to authenticated with check (user_id = (select auth.uid()));

create policy "users remove own favorites" on public.favorites
  for delete to authenticated using (user_id = (select auth.uid()));

create policy "users read own history" on public.reading_history
  for select to authenticated using (user_id = (select auth.uid()));

create policy "users add own history" on public.reading_history
  for insert to authenticated with check (user_id = (select auth.uid()));

create policy "users update own history" on public.reading_history
  for update to authenticated
  using (user_id = (select auth.uid()))
  with check (user_id = (select auth.uid()));

create policy "users delete own history" on public.reading_history
  for delete to authenticated using (user_id = (select auth.uid()));
