ALTER TABLE feed ADD COLUMN greader_hidden INTEGER NOT NULL DEFAULT 0
    CHECK (greader_hidden IN (0, 1));
