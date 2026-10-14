// Supabase Edge Function `sync`: see handler.ts.

import { createClient } from 'npm:@supabase/supabase-js@2.117.2';
import { handle } from './handler.ts';
import { loadStorage } from './storage.ts';

const auth = createClient(Deno.env.get('SUPABASE_URL')!, Deno.env.get('SUPABASE_ANON_KEY')!, {
  auth: { persistSession: false, autoRefreshToken: false }
});

Deno.serve(req =>
  handle(req, {
    async userId(token) {
      const { data, error } = await auth.auth.getUser(token);
      return error ? null : data.user?.id ?? null;
    },
    // Read on each call, so changed secrets apply at once.
    storage: () => loadStorage()
  })
);
