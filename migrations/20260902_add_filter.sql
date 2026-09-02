-- User-defined filter rules that exclude matching articles from read paths.
--
-- article_filter_match is a materialized join, kept in sync incrementally at
-- ingestion/filter-edit time rather than recomputed on every read. `enabled`
-- is checked at query time via a join, never by deleting match rows, so
-- disabling a filter is a one-column UPDATE and re-enabling it retroactively
-- honours matches recorded while it was off.

CREATE TABLE filter (
    pk         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL,
    field      TEXT    NOT NULL DEFAULT 'both' CHECK (field IN ('title', 'content', 'both')),
    match_type TEXT    NOT NULL CHECK (match_type IN ('contains', 'regex')),
    pattern    TEXT    NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE article_filter_match (
    article INTEGER NOT NULL REFERENCES article (pk) ON DELETE CASCADE,
    filter  INTEGER NOT NULL REFERENCES filter (pk)  ON DELETE CASCADE,
    PRIMARY KEY (article, filter)
) STRICT;

CREATE INDEX idx_article_filter_match_filter ON article_filter_match (filter);
