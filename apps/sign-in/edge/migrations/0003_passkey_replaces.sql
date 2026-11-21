-- The passkey a new one replaces, which can't unlock its person's key: it's removed once the new one does (`auth.ts`).
alter table "passkey" add column "replaces" text;
