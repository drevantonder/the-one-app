-- What signs the sign-in Worker's cookies and tokens (`secret()` in `auth.ts`), made once, as 64 hex digits.
create table if not exists secret (secret text not null);
insert into secret select lower(hex(randomblob(32))) where not exists (select 1 from secret);
-- Which version of the Worker last made Better Auth's tables, when it made them as it started: these migrations do now.
drop table if exists migrated;
