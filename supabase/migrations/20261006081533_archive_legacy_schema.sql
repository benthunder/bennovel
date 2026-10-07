-- Park the first draft schema (2026-09-27) that was created directly on the
-- hosted project. It held no data; its objects are moved into a private
-- `legacy_20260927` schema (not exposed through the API) instead of dropped, so
-- the names are free for the migrations that follow. Drop that schema by hand
-- once nobody needs to look at it.
-- On a fresh local database none of these objects exist and this is a no-op.

create schema if not exists legacy_20260927;
revoke all on schema legacy_20260927 from public, anon, authenticated;

do $$
declare
  t text;
  f text;
begin
  foreach t in array array['users', 'novels', 'categories', 'authors', 'novel_categories',
                           'novel_authors', 'novel_chapters', 'novel_metadata'] loop
    if to_regclass('public.' || t) is not null then
      execute format('alter table public.%I set schema legacy_20260927', t);
    end if;
  end loop;

  foreach f in array array['handle_new_auth_user()', 'prevent_user_type_change()',
                           'trg_novel_chapters_recalc_metadata()', 'current_app_user_id()',
                           'is_admin_or_system()', 'recalc_novel_chapter_count(uuid, chapter_language)'] loop
    if to_regprocedure('public.' || f) is not null then
      execute format('alter function public.%s set schema legacy_20260927', f);
    end if;
  end loop;

  foreach t in array array['user_type', 'content_status', 'approval_status', 'chapter_language'] loop
    if to_regtype('public.' || t) is not null then
      execute format('alter type public.%I set schema legacy_20260927', t);
    end if;
  end loop;

  -- The old sign-up trigger on auth.users stays in place (it belongs to the auth
  -- schema owner), but its function becomes a no-op.
  if to_regprocedure('legacy_20260927.handle_new_auth_user()') is not null then
    create or replace function legacy_20260927.handle_new_auth_user()
    returns trigger language plpgsql set search_path = '' as $fn$ begin return new; end; $fn$;
  end if;
end;
$$;
