-- The network's settings for signing in, each a JSON value by its name: `domains`, the Google Workspace domains whose
-- people join by signing in with Google (`asked` in `auth.ts`).
create table if not exists setting (name text not null primary key, value text not null);
