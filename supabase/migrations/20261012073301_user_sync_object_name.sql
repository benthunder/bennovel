-- Each upload gets its own object name, recorded on commit, so two devices uploading
-- at once never write the same object.
alter table public.user_sync add column object text not null check (object <> '');

-- The first commit_sync (no object name) is retired: renamed and closed to callers.
alter function public.commit_sync(bigint, bigint) rename to commit_sync_retired;
revoke execute on function public.commit_sync_retired(bigint, bigint) from public, anon, authenticated;

-- Moves the caller's file from version `expected` (0 = none yet) to the next one,
-- stored at `new_object` (in the caller's folder). Returns the new version, or null
-- when another device synced first.
create function public.commit_sync(expected bigint, new_object text, new_size bigint)
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
  if split_part(new_object, '/', 1) <> uid::text then
    raise exception 'object must be in the caller''s folder' using errcode = '42501';
  end if;
  if expected = 0 then
    insert into public.user_sync (user_id, version, object, size_bytes)
    values (uid, 1, new_object, new_size)
    on conflict (user_id) do nothing
    returning version into next_version;
  else
    update public.user_sync
    set version = version + 1, object = new_object, size_bytes = new_size, updated_at = now()
    where user_id = uid and version = expected
    returning version into next_version;
  end if;
  return next_version;
end;
$$;

revoke execute on function public.commit_sync(bigint, text, bigint) from public, anon;
grant execute on function public.commit_sync(bigint, text, bigint) to authenticated;
