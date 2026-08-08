-- Initial schema: feeds, categories, articles, states, labels.
--
-- Conventions:
--   * uuid       -> TEXT (36-char canonical form), UNIQUE
--   * json       -> TEXT validated with json_valid()
--   * timestamp  -> INTEGER, Unix epoch seconds (UTC)
--   * bool       -> INTEGER constrained to 0/1
--
-- NOTE: foreign_keys is a per-connection pragma, not a schema property.
-- Your application must issue `PRAGMA foreign_keys = ON;` on every connection
-- or the FK clauses below are inert.

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- feed
-- ---------------------------------------------------------------------------
CREATE TABLE feed (
    pk               INTEGER PRIMARY KEY AUTOINCREMENT,
    id               TEXT    NOT NULL UNIQUE,
    name             TEXT    NOT NULL,
    url              TEXT    NOT NULL UNIQUE,
    metadata         TEXT    NOT NULL DEFAULT '{}'
                             CHECK (json_valid(metadata)),
    refresh_interval INTEGER NOT NULL DEFAULT 3600
                             CHECK (refresh_interval > 0),
    last_refresh     INTEGER          DEFAULT NULL,
    next_poll_at     INTEGER          DEFAULT NULL
) STRICT;

-- Scheduler lookup: "which feeds are due for a refresh?"
CREATE INDEX idx_feed_next_poll_at ON feed (next_poll_at);

-- ---------------------------------------------------------------------------
-- category (folder)
-- ---------------------------------------------------------------------------
CREATE TABLE category (
    pk   INTEGER PRIMARY KEY AUTOINCREMENT,
    id   TEXT    NOT NULL UNIQUE,
    name TEXT    NOT NULL UNIQUE
) STRICT;

-- ---------------------------------------------------------------------------
-- feed_category (many-to-many)
-- ---------------------------------------------------------------------------
CREATE TABLE feed_category (
    pk       INTEGER PRIMARY KEY AUTOINCREMENT,
    feed     INTEGER NOT NULL REFERENCES feed (pk)     ON DELETE CASCADE,
    category INTEGER NOT NULL REFERENCES category (pk) ON DELETE CASCADE,
    UNIQUE (feed, category)
) STRICT;

-- The UNIQUE(feed, category) index already covers lookups by feed;
-- this one covers the reverse direction ("feeds in this category").
CREATE INDEX idx_feed_category_category ON feed_category (category);

-- ---------------------------------------------------------------------------
-- article
-- ---------------------------------------------------------------------------
CREATE TABLE article (
    pk           INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT    NOT NULL UNIQUE,
    feed         INTEGER NOT NULL REFERENCES feed (pk) ON DELETE CASCADE,
    url          TEXT    NOT NULL UNIQUE,
    author       TEXT    NOT NULL DEFAULT '',
    content      TEXT    NOT NULL DEFAULT '',
    published_at INTEGER          DEFAULT NULL,
    retrieved_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- Primary read path: a feed's articles, newest first.
CREATE INDEX idx_article_feed_published ON article (feed, published_at DESC);
CREATE INDEX idx_article_published      ON article (published_at DESC);

-- ---------------------------------------------------------------------------
-- article_state (0..1 per article)
-- ---------------------------------------------------------------------------
CREATE TABLE article_state (
    pk          INTEGER PRIMARY KEY AUTOINCREMENT,
    article     INTEGER NOT NULL UNIQUE REFERENCES article (pk) ON DELETE CASCADE,
    is_read     INTEGER NOT NULL DEFAULT 0 CHECK (is_read    IN (0, 1)),
    is_starred  INTEGER NOT NULL DEFAULT 0 CHECK (is_starred IN (0, 1)),
    modified_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- Partial indexes: unread/starred sets are small relative to the table.
CREATE INDEX idx_article_state_unread  ON article_state (article) WHERE is_read = 0;
CREATE INDEX idx_article_state_starred ON article_state (article) WHERE is_starred = 1;

-- Keep modified_at honest without relying on the application layer.
CREATE TRIGGER trg_article_state_touch
AFTER UPDATE ON article_state
FOR EACH ROW
WHEN NEW.modified_at = OLD.modified_at
BEGIN
    UPDATE article_state
       SET modified_at = unixepoch()
     WHERE pk = NEW.pk;
END;

-- ---------------------------------------------------------------------------
-- label
-- ---------------------------------------------------------------------------
CREATE TABLE label (
    pk   INTEGER PRIMARY KEY AUTOINCREMENT,
    id   TEXT    NOT NULL UNIQUE,
    name TEXT    NOT NULL UNIQUE
) STRICT;

-- ---------------------------------------------------------------------------
-- article_label (many-to-many)
-- ---------------------------------------------------------------------------
CREATE TABLE article_label (
    pk      INTEGER PRIMARY KEY AUTOINCREMENT,
    article INTEGER NOT NULL REFERENCES article (pk) ON DELETE CASCADE,
    label   INTEGER NOT NULL REFERENCES label (pk)   ON DELETE CASCADE,
    UNIQUE (article, label)
) STRICT;

CREATE INDEX idx_article_label_label ON article_label (label);
