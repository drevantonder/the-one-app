-- An app, by where its code is: apps/<name>/ in github.com/<owner>/<repo>.
CREATE TABLE IF NOT EXISTS apps (
  owner TEXT NOT NULL,
  repo TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT NOT NULL, -- its SKILL.md's, for agents
  subtitle TEXT, -- its metadata's, for people: a few words under its name
  about TEXT, -- its metadata's, for people: a few sentences on its page
  category TEXT, -- its metadata's: what it's for, as the building-apps skill lists them
  listing TEXT, -- JSON: its title, icon, entrypoints, widgets and requires, as a network that installed it said (worker.ts' Listing)
  checked TEXT NOT NULL, -- when its SKILL.md was last read, ISO 8601
  PRIMARY KEY (owner, repo, name)
);

-- How many times an app was installed on a day (UTC, YYYY-MM-DD). Nothing about who installed it.
CREATE TABLE IF NOT EXISTS installs (
  owner TEXT NOT NULL,
  repo TEXT NOT NULL,
  name TEXT NOT NULL,
  day TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (owner, repo, name, day)
);
