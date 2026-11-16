-- A plugin, by where its code is: plugins/<name>/ in github.com/<owner>/<repo>.
CREATE TABLE IF NOT EXISTS plugins (
  owner TEXT NOT NULL,
  repo TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT NOT NULL,
  checked TEXT NOT NULL, -- when its SKILL.md was last read, ISO 8601
  PRIMARY KEY (owner, repo, name)
);

-- How many times a plugin was installed on a day (UTC, YYYY-MM-DD). Nothing about who installed it.
CREATE TABLE IF NOT EXISTS installs (
  owner TEXT NOT NULL,
  repo TEXT NOT NULL,
  name TEXT NOT NULL,
  day TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (owner, repo, name, day)
);
