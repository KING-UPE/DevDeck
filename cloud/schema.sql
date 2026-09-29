-- DevDeck cloud schema.
--
-- Run this once in your Supabase project: SQL Editor -> New query -> paste ->
-- Run. It is idempotent, so re-running after an upgrade is safe.
--
-- WHAT LIVES HERE
--
--   auth.users   email, password hash, verification state  (managed by Supabase)
--   devices      a machine's name and current tunnel URL
--
-- WHAT DELIBERATELY DOES NOT
--
--   Workspaces, project paths, custom names and logs stay on the user's own
--   answering "where is this account's PC right now".

create table if not exists public.devices (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null references auth.users (id) on delete cascade,
    name        text not null,
    tunnel_url  text,
    updated_at  timestamptz not null default now(),

    -- One row per machine per account, so re-registering updates rather than
    -- piling up duplicates. The client's upsert relies on this constraint.
    unique (user_id, name)
);

create index if not exists devices_user_id_idx on public.devices (user_id);

-- Keep updated_at honest; clients must not be trusted to set it.
create or replace function public.touch_updated_at()
returns trigger
language plpgsql
as $$
begin
    new.updated_at = now();
    return new;
end;
$$;

drop trigger if exists devices_touch_updated_at on public.devices;
create trigger devices_touch_updated_at
    before update on public.devices
    for each row execute function public.touch_updated_at();

-- ---------------------------------------------------------------------------
-- Row Level Security
--
-- This is what actually isolates users. Without it the anon key, which ships
-- in every client and is meant to be public, would expose every row. With it,
-- Postgres refuses to return another account's devices no matter what the
-- client asks for.
-- ---------------------------------------------------------------------------

alter table public.devices enable row level security;

drop policy if exists "read own devices"   on public.devices;
drop policy if exists "insert own devices" on public.devices;
drop policy if exists "update own devices" on public.devices;
drop policy if exists "delete own devices" on public.devices;

create policy "read own devices"
    on public.devices for select
    using (auth.uid() = user_id);

-- `with check` on insert stops a client writing a row that claims to belong to
-- somebody else.
create policy "insert own devices"
    on public.devices for insert
    with check (auth.uid() = user_id);

create policy "update own devices"
    on public.devices for update
    using (auth.uid() = user_id)
    with check (auth.uid() = user_id);

create policy "delete own devices"
    on public.devices for delete
    using (auth.uid() = user_id);

-- ---------------------------------------------------------------------------
-- Housekeeping
--
-- A machine that stops publishing should stop being advertised. Schedule this
-- from Database -> Cron if you want it automatic; it is not required.
-- ---------------------------------------------------------------------------

create or replace function public.prune_stale_devices()
returns void
language sql
security definer
set search_path = public
as $$
    update public.devices
       set tunnel_url = null
     where tunnel_url is not null
       and updated_at < now() - interval '7 days';
$$;
