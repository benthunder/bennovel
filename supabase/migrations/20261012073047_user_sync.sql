-- Sync between a user's devices. The whole on-device library (a SQLite file, see
-- src-tauri/src/library/sync.rs) is kept in Storage, one folder per user:
--   user-sync/{user_id}/library-….sqlite.gz (one object; see user_sync.object)
-- Favourites and history live only in that file, never in database tables.

-- ---------------------------------------------------------------------------
-- The current version of each user's file. A device uploads the next version, then
-- commits it here; the commit only succeeds when nobody synced in between, so two
-- devices never overwrite each other's changes.
-- ---------------------------------------------------------------------------

create table public.user_sync (
  user_id    uuid primary key default auth.uid() references auth.users (id) on delete cascade,
  version    bigint not null check (version > 0),
  size_bytes bigint not null default 0 check (size_bytes >= 0),
  updated_at timestamptz not null default now()
);

alter table public.user_sync enable row level security;

-- Written only through commit_sync().
create policy "users read own sync" on public.user_sync
  for select to authenticated using (user_id = (select auth.uid()));

-- Moves the caller's file from version `expected` (0 = none yet) to the next one.
-- Returns the new version, or null when another device synced first.
create or replace function public.commit_sync(expected bigint, new_size bigint)
returns bigint
language plpgsql
security definer
set search_path = ''
as $$
declare
  uid uuid := auth.uid();
  next_version bigint;
begin
  if uid is null then
    raise exception 'not signed in' using errcode = '42501';
  end if;
  if expected = 0 then
    insert into public.user_sync (user_id, version, size_bytes)
    values (uid, 1, new_size)
    on conflict (user_id) do nothing
    returning version into next_version;
  else
    update public.user_sync
    set version = version + 1, size_bytes = new_size, updated_at = now()
    where user_id = uid and version = expected
    returning version into next_version;
  end if;
  return next_version;
end;
$$;

revoke execute on function public.commit_sync(bigint, bigint) from public, anon;
grant execute on function public.commit_sync(bigint, bigint) to authenticated;

-- ---------------------------------------------------------------------------
-- Storage: a private bucket; each user reaches only their own folder.
-- ---------------------------------------------------------------------------

insert into storage.buckets (id, name, public)
values ('user-sync', 'user-sync', false)
on conflict (id) do nothing;

create policy "user-sync read own folder" on storage.objects
  for select to authenticated
  using (bucket_id = 'user-sync' and (storage.foldername(name))[1] = (select auth.uid())::text);

create policy "user-sync add to own folder" on storage.objects
  for insert to authenticated
  with check (bucket_id = 'user-sync' and (storage.foldername(name))[1] = (select auth.uid())::text);

create policy "user-sync replace in own folder" on storage.objects
  for update to authenticated
  using (bucket_id = 'user-sync' and (storage.foldername(name))[1] = (select auth.uid())::text)
  with check (bucket_id = 'user-sync' and (storage.foldername(name))[1] = (select auth.uid())::text);

create policy "user-sync remove from own folder" on storage.objects
  for delete to authenticated
  using (bucket_id = 'user-sync' and (storage.foldername(name))[1] = (select auth.uid())::text);
