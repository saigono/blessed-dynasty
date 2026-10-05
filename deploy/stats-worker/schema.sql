-- Stage 11b: the games the game reports at their end (worker.js). Apply with
-- `wrangler d1 execute bd-stats --remote --file schema.sql` (docs/stats.md).
CREATE TABLE IF NOT EXISTS games (
    id INTEGER PRIMARY KEY,
    date TEXT NOT NULL DEFAULT (datetime('now')),
    version TEXT NOT NULL,
    link TEXT NOT NULL,
    fall TEXT NOT NULL,
    years INTEGER NOT NULL,
    score INTEGER NOT NULL
);
