-- Remove the first draft schema (2026-09-27) that was created directly on the
-- hosted project. It held no data and is replaced by the migrations that follow.
-- Everything is "if exists" so a fresh local database runs this as a no-op.

drop trigger if exists on_auth_user_created on auth.users;

drop table if exists
  public.novel_metadata,
  public.novel_chapters,
  public.novel_categories,
  public.novel_authors,
  public.novels,
  public.categories,
  public.authors,
  public.users
  cascade;

drop function if exists public.handle_new_auth_user();
drop function if exists public.prevent_user_type_change();
drop function if exists public.trg_novel_chapters_recalc_metadata();
drop function if exists public.current_app_user_id();
drop function if exists public.is_admin_or_system();

do $$
begin
  if to_regtype('public.chapter_language') is not null then
    drop function if exists public.recalc_novel_chapter_count(uuid, public.chapter_language);
  end if;
end;
$$;

drop type if exists public.user_type;
drop type if exists public.content_status;
drop type if exists public.approval_status;
drop type if exists public.chapter_language;
