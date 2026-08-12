-- author is unreliable per RSS/Atom spec (often absent) and unused — drop it.
-- title is near-universal across RSS/Atom — capture it.
-- summary is a short excerpt distinct from full content — capture it separately.
-- guid replaces url as the dedup/identity key: it's the spec-intended stable
-- identifier and survives link changes (redirects, tracking params, site
-- restructuring) that would otherwise mint duplicate rows under url-based dedup.
--
-- SQLite >= 3.35 supports DROP COLUMN natively, including on STRICT tables;
-- sqlx 0.9's bundled libsqlite3-sys is recent enough. NOT NULL columns can be
-- added via ALTER TABLE as long as a constant DEFAULT is given (only a NOT
-- NULL column *without* a default is disallowed on a non-empty table).

ALTER TABLE article DROP COLUMN author;

ALTER TABLE article ADD COLUMN title   TEXT DEFAULT NULL;
ALTER TABLE article ADD COLUMN summary TEXT DEFAULT NULL;
ALTER TABLE article ADD COLUMN guid    TEXT NOT NULL DEFAULT '';

-- Backfill: existing rows have no real guid concept yet, and url was already
-- the de facto identity key (UNIQUE), so it's a safe one-time backfill value.
UPDATE article SET guid = url WHERE guid = '';

-- guid becomes the identity/upsert key going forward.
CREATE UNIQUE INDEX idx_article_guid ON article (guid);
