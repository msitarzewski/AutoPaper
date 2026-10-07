//! SQLite storage: keywords, settings, generations (the agent's memory), taste, spend, and a small
//! key-value table for engine state (backoff, failure counts). One file, `autopaper.sqlite3`.
//!
//! Schema in `memory-bank/systemPatterns.md` ("Storage"). Migrations by `PRAGMA user_version`; each
//! migration runs in a transaction. WAL mode, `foreign_keys = ON`, `busy_timeout` 5 s. Embeddings are
//! stored as little-endian f32 BLOBs. Concepts and keyword snapshots as JSON text. Settings as one JSON
//! row per key, so new settings get defaults when absent (forward/backward compatible).
//!
//! Compatibility rules, so an older build keeps working on a newer build's database (a downgrade):
//! - Migrations only add (tables, nullable or defaulted columns, indexes) and are never edited once
//!   shipped. A database newer than this build opens as it is, without migrating.
//! - Enum columns hold stable snake_case names, the same strings serde writes into settings and keyword
//!   snapshots. They have no CHECK constraint, so a new variant needs no table rebuild; a name this build
//!   doesn't know reads back as a `Storage` error for that row.
//! - Settings keys and concept fields this build doesn't know are ignored on read and left in place;
//!   ones it knows but finds missing or unreadable take their defaults, one field at a time.
//! - `echo_of` is a soft reference (no foreign key): echoes keep their original's id after it is deleted.
//! - `hidden` marks generations cleared from history while their memory is kept (`clear_history(true)`):
//!   history, `current` and `lineage` leave them out; memory, novelty and summaries still see them.
//! - `least_similar` (migration 2) marks generations whose compose loop found nothing novel enough and took
//!   the least similar valid candidate; `recent_least_similar` feeds `Engine::keywords_are_narrow`.
//! - Moods (migration 3): `moods(id, name UNIQUE NOCASE, position, surprise, created_at)`; every keyword belongs
//!   to one (`keywords.mood_id`, cascade on delete; keyword text is unique per mood, not overall — the one
//!   migration that rebuilds a table, since SQLite can't drop a column's UNIQUE); `generations.mood_id` (a soft
//!   reference: a deleted mood leaves its wallpapers' id behind, and `mood_name` reads `None`); the active mood's
//!   id in state (`active_mood`; missing or deleted → the first mood). The migration turns the keywords and
//!   Settings' Surprise into the first mood (named from its first two keywords, else "My mood") and files every
//!   existing wallpaper under it. Keywords without a mood (written by an older build after a downgrade) join the
//!   active mood when the store opens. `settings()` reads Surprise from the active mood and `save_settings`
//!   writes it there (and to the settings row, for older builds).
//! - Timings (migration 4): one row per finished provider call — job, provider, server origin, model, size,
//!   steps, seconds, finished_at — newest `TIMINGS_KEPT` per (job, provider, origin, model); `perf` turns them
//!   into estimates.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, Params, Row, Transaction, TransactionBehavior, named_params, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::{AutoPaperError, InvalidInputReason, Result};
use crate::model::{
    Concept, DayCount, Generation, GenerationStatus, HistoryFilter, Keyword, KeywordSnapshot, KeywordWeight, Mood,
    MoodStats, ProviderJob, ProviderKind, Rating, Settings, Trigger,
};
use crate::perf::Timing;
use crate::taste::TasteRow;
use crate::text::{self, normalize_keyword, normalize_mood_name};

/// A generation plus what only the core needs (never crosses the FFI).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredGeneration {
    pub generation: Generation,
    /// Unit-length embedding of `embed::concept_text(&generation.concept)`; empty for failed generations.
    pub embedding: Vec<f32>,
    pub embedding_model: String,
    /// 64-bit perceptual hash of the image; `None` for failed generations.
    pub phash: Option<u64>,
    /// No candidate was novel enough, even after asking again: this is the least similar valid one
    /// (`Engine::keywords_are_narrow` counts these).
    pub least_similar: bool,
}

/// The slice of a generation that novelty and echoes need, loaded for all of history at once.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRow {
    pub id: String,
    pub created_at: i64,
    pub title: String,
    pub summary: String,
    pub embedding: Vec<f32>,
    pub embedding_model: String,
    pub rating: Rating,
    pub echo_of: Option<String>,
    /// The mood it was made under (echoes prefer the active mood's originals).
    pub mood_id: Option<String>,
}

/// One connection to `autopaper.sqlite3`. Reads take `&self`; writes take `&mut self` and run in an
/// IMMEDIATE transaction, so another process using the same file waits (up to the busy timeout)
/// instead of interleaving.
pub struct Store {
    conn: rusqlite::Connection,
}

/// How long a statement waits for another connection's lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Schema migrations, in order; `PRAGMA user_version` is the number applied. Append only.
const MIGRATIONS: &[Migration] = &[
    Migration { sql: MIGRATION_1, data: None },
    Migration { sql: MIGRATION_2, data: None },
    Migration { sql: MIGRATION_3, data: Some(first_mood) },
    Migration { sql: MIGRATION_4, data: None },
];

/// One schema step: SQL, then (in the same transaction) an optional data step for what SQL can't say well.
#[derive(Clone, Copy)]
struct Migration {
    sql: &'static str,
    data: Option<fn(&Connection) -> Result<()>>,
}

/// State key: the active mood's id.
const STATE_ACTIVE_MOOD: &str = "active_mood";
/// Timing rows kept per (job, provider, origin, model): the estimate weighs recent ones most anyway.
pub const TIMINGS_KEPT: u32 = 50;

const MIGRATION_1: &str = r#"
CREATE TABLE keywords (
    id         TEXT PRIMARY KEY NOT NULL,
    text       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    weight     TEXT NOT NULL,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE generations (
    id              TEXT PRIMARY KEY NOT NULL,
    created_at      INTEGER NOT NULL,
    "trigger"       TEXT NOT NULL,
    status          TEXT NOT NULL,
    title           TEXT NOT NULL,
    summary         TEXT NOT NULL,
    concept_json    TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    text_provider   TEXT NOT NULL,
    text_model      TEXT NOT NULL,
    image_provider  TEXT NOT NULL,
    image_model     TEXT NOT NULL,
    width           INTEGER NOT NULL,
    height          INTEGER NOT NULL,
    image_path      TEXT,
    thumb_path      TEXT,
    phash           INTEGER,
    embedding       BLOB NOT NULL,
    embedding_model TEXT NOT NULL,
    echo_of         TEXT,
    echo_note       TEXT,
    surprise        REAL NOT NULL,
    keywords_json   TEXT NOT NULL,
    rating          INTEGER NOT NULL DEFAULT 0 CHECK (rating IN (-1, 0, 1)),
    rated_at        INTEGER,
    last_shown_at   INTEGER,
    shown_count     INTEGER NOT NULL DEFAULT 0,
    cost_microusd   INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    hidden          INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1))
) STRICT;

-- History pages, memory, recent summaries and the last scheduled time: status = 'ok', by time.
CREATE INDEX generations_by_status_time ON generations (status, created_at);
-- Liked/Disliked history pages, revisits, the rated count.
CREATE INDEX generations_by_rating ON generations (rating, status, created_at);
-- Lineage walks and the Echoes filter.
CREATE INDEX generations_by_echo_of ON generations (echo_of) WHERE echo_of IS NOT NULL;
-- The wallpaper showing now.
CREATE INDEX generations_by_last_shown ON generations (last_shown_at) WHERE last_shown_at IS NOT NULL;

CREATE TABLE taste (
    feature    TEXT PRIMARY KEY NOT NULL,
    likes      REAL NOT NULL,
    dislikes   REAL NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE spend (
    month    TEXT PRIMARY KEY NOT NULL,
    microusd INTEGER NOT NULL DEFAULT 0,
    images   INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE state (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
"#;

/// The compose loop took the least similar valid candidate (nothing was novel enough). Rows from before it
/// read 0: unknown, and treated as novel.
const MIGRATION_2: &str = r#"
ALTER TABLE generations ADD COLUMN least_similar INTEGER NOT NULL DEFAULT 0 CHECK (least_similar IN (0, 1));
"#;

/// Moods. Keywords move into them (the table is rebuilt so that text is unique per mood); generations note theirs.
/// `first_mood` then makes the first mood from the keywords and Settings' Surprise.
const MIGRATION_3: &str = r#"
CREATE TABLE moods (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    position   INTEGER NOT NULL,
    surprise   REAL NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE keywords_in_moods (
    id         TEXT PRIMARY KEY NOT NULL,
    mood_id    TEXT REFERENCES moods (id) ON DELETE CASCADE,
    text       TEXT NOT NULL COLLATE NOCASE,
    weight     TEXT NOT NULL,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (mood_id, text)
) STRICT;
INSERT INTO keywords_in_moods (id, mood_id, text, weight, position, created_at)
    SELECT id, NULL, text, weight, position, created_at FROM keywords;
DROP TABLE keywords;
ALTER TABLE keywords_in_moods RENAME TO keywords;

ALTER TABLE generations ADD COLUMN mood_id TEXT;
-- History filtered by mood, a mood's recent wallpapers, narrow keywords per mood.
CREATE INDEX generations_by_mood ON generations (mood_id, status, created_at) WHERE mood_id IS NOT NULL;
"#;

/// Performance history: how long each finished provider call took.
const MIGRATION_4: &str = r#"
CREATE TABLE timings (
    id          INTEGER PRIMARY KEY,
    job         TEXT NOT NULL,
    provider    TEXT NOT NULL,
    origin      TEXT NOT NULL,
    model       TEXT NOT NULL,
    width       INTEGER NOT NULL,
    height      INTEGER NOT NULL,
    steps       INTEGER,
    seconds     REAL NOT NULL,
    finished_at INTEGER NOT NULL
) STRICT;
CREATE INDEX timings_by_key ON timings (job, provider, origin, model, finished_at);
"#;

/// The columns `generation_from_row` reads (`mood_name` is the mood's name now, `NULL` once it's deleted).
macro_rules! generation_columns {
    () => {
        r#"id, created_at, "trigger", status, concept_json, image_path, thumb_path, width, height, rating,
        echo_of, echo_note, text_provider, text_model, image_provider, image_model, surprise, keywords_json,
        cost_microusd, last_shown_at, shown_count, error, mood_id,
        (SELECT name FROM moods WHERE moods.id = generations.mood_id) AS mood_name"#
    };
}

/// One page of history: successful generations matching `$condition`, newest first.
macro_rules! history_query {
    ($condition:literal) => {
        concat!(
            "SELECT ",
            generation_columns!(),
            " FROM generations WHERE status = 'ok' AND hidden = 0",
            $condition,
            " ORDER BY created_at DESC, rowid DESC LIMIT ?1 OFFSET ?2"
        )
    };
}

/// `history_query!` made under one mood (`?3`).
macro_rules! mood_history_query {
    ($condition:literal) => {
        concat!(
            "SELECT ",
            generation_columns!(),
            " FROM generations WHERE mood_id = ?3 AND status = 'ok' AND hidden = 0",
            $condition,
            " ORDER BY created_at DESC, rowid DESC LIMIT ?1 OFFSET ?2"
        )
    };
}

/// What `mood_stats` counts: History's wallpapers (finished, not cleared from History) of moods that still exist.
macro_rules! mood_stats_rows {
    () => {
        "mood_id IN (SELECT id FROM moods) AND status = 'ok' AND hidden = 0"
    };
}

/// Per mood: wallpapers, liked, disliked, echoes, the newest one's time.
const MOOD_COUNTS: &str = concat!(
    "SELECT mood_id, COUNT(*), COUNT(CASE WHEN rating = 1 THEN 1 END), COUNT(CASE WHEN rating = -1 THEN 1 END),
     COUNT(echo_of), MAX(created_at) FROM generations WHERE ",
    mood_stats_rows!(),
    " GROUP BY mood_id"
);

/// Each mood's newest `?1` wallpapers, newest first (History's order).
const MOOD_LATEST: &str = concat!(
    "SELECT * FROM (SELECT ",
    generation_columns!(),
    ", ROW_NUMBER() OVER (PARTITION BY mood_id ORDER BY created_at DESC, rowid DESC) AS mood_rank
     FROM generations WHERE ",
    mood_stats_rows!(),
    ") WHERE mood_rank <= ?1 ORDER BY mood_id, mood_rank"
);

const INSERT_GENERATION: &str = r#"
INSERT INTO generations (
    id, created_at, "trigger", status, title, summary, concept_json, prompt,
    text_provider, text_model, image_provider, image_model, width, height, image_path, thumb_path,
    phash, embedding, embedding_model, echo_of, echo_note, surprise, keywords_json,
    rating, rated_at, last_shown_at, shown_count, cost_microusd, error, least_similar, mood_id
) VALUES (
    :id, :created_at, :trigger, :status, :title, :summary, :concept_json, :prompt,
    :text_provider, :text_model, :image_provider, :image_model, :width, :height, :image_path, :thumb_path,
    :phash, :embedding, :embedding_model, :echo_of, :echo_note, :surprise, :keywords_json,
    :rating, :rated_at, :last_shown_at, :shown_count, :cost_microusd, :error, :least_similar, :mood_id
)"#;

impl Store {
    /// Opens (creating and migrating) the database at `path`. Its directory must exist.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        let mode: String = conn.pragma_update_and_check(None, "journal_mode", "wal", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            // Some file systems can't share WAL memory; rollback journaling is slower but just as safe.
            tracing::warn!(journal_mode = %mode, "SQLite couldn't switch to WAL");
        }
        Self::configure(conn)
    }

    /// A private database that disappears when dropped (tests, `autopaper simulate`).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Self::configure(conn)
    }

    fn configure(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn, MIGRATIONS)?;
        let mut store = Self { conn };
        store.settle_moods()?;
        Ok(store)
    }

    /// There is always a mood, and every keyword has one: a database without moods gets "My mood" (a newer
    /// build's, say, that removed them all), and keywords without a mood (an older build wrote them after a
    /// downgrade, or their mood is gone) join the active one — those it already has are dropped. Writes only
    /// when something is off.
    fn settle_moods(&mut self) -> Result<()> {
        const ORPHANS: &str = "mood_id IS NULL OR mood_id NOT IN (SELECT id FROM moods)";
        let (moods, orphans): (u32, u32) = self.conn.query_row(
            &format!("SELECT (SELECT COUNT(*) FROM moods), (SELECT COUNT(*) FROM keywords WHERE {ORPHANS})"),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if moods > 0 && orphans == 0 {
            return Ok(());
        }
        let tx = self.write()?;
        if moods == 0 {
            insert_mood(&tx, text::DEFAULT_MOOD_NAME, 0, Settings::default().surprise, unix_now())?;
        }
        let active = active_mood_id(&tx)?;
        tx.execute(&format!("UPDATE OR IGNORE keywords SET mood_id = ?1 WHERE {ORPHANS}"), [&active])?;
        let dropped = tx.execute(&format!("DELETE FROM keywords WHERE {ORPHANS}"), [])?;
        if dropped > 0 {
            tracing::info!(dropped, "keywords without a mood duplicated the active mood's; dropped");
        }
        let ids: Vec<String> = load_keywords(&tx, &active)?.into_iter().map(|keyword| keyword.id).collect();
        renumber(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// An IMMEDIATE transaction: takes the write lock up front, so a read-then-write can't race
    /// another connection (and never fails half way with SQLITE_BUSY on lock upgrade).
    fn write(&mut self) -> Result<Transaction<'_>> {
        Ok(self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
    }

    // ── Moods ───────────────────────────────────────────────────────────────────────────────

    /// Every mood in the person's order, with its keywords; the active one marked.
    pub fn moods(&self) -> Result<Vec<Mood>> {
        let active = active_mood_id(&self.conn)?;
        load_moods(&self.conn)?.into_iter().map(|row| row.into_mood(&self.conn, &active)).collect()
    }

    /// `None` for an unknown id.
    pub fn mood(&self, id: &str) -> Result<Option<Mood>> {
        let active = active_mood_id(&self.conn)?;
        load_moods(&self.conn)?.into_iter().find(|row| row.id == id).map(|row| row.into_mood(&self.conn, &active)).transpose()
    }

    /// The mood in use: the one named in state, else (missing or deleted) the first.
    pub fn active_mood(&self) -> Result<Mood> {
        let id = active_mood_id(&self.conn)?;
        self.mood(&id)?.ok_or_else(|| internal("the active mood vanished"))
    }

    pub fn active_mood_id(&self) -> Result<String> {
        active_mood_id(&self.conn)
    }

    /// A new mood at the end of the list. With `copy_from`, a duplicate of that mood: its keywords (new ids, same
    /// order) and its Surprise; without, no keywords and the default Surprise. `InvalidInput` for a name that's
    /// empty, too long or taken (case-insensitive); `NotFound` for an unknown `copy_from`.
    pub fn create_mood(&mut self, name: &str, copy_from: Option<&str>, now: i64) -> Result<Mood> {
        let name = normalize_mood_name(name)?;
        let tx = self.write()?;
        let moods = load_moods(&tx)?;
        check_mood_name(&moods, None, &name)?;
        let source = match copy_from {
            Some(id) => Some(moods.iter().find(|row| row.id == id).ok_or(AutoPaperError::NotFound)?),
            None => None,
        };
        let surprise = source.map_or(Settings::default().surprise, |row| row.surprise);
        let id = insert_mood(&tx, &name, position(moods.len())?, surprise, now)?;
        if let Some(source) = source {
            for keyword in load_keywords(&tx, &source.id)? {
                insert_keyword(&tx, &id, &Keyword { id: Uuid::now_v7().to_string(), created_at: now, ..keyword })?;
            }
        }
        tx.commit()?;
        self.mood(&id)?.ok_or_else(|| internal("the new mood vanished"))
    }

    /// `NotFound` for an unknown id; `InvalidInput` as for `create_mood` (its own name in another case is fine).
    pub fn rename_mood(&mut self, id: &str, name: &str) -> Result<Mood> {
        let name = normalize_mood_name(name)?;
        let tx = self.write()?;
        let moods = load_moods(&tx)?;
        if !moods.iter().any(|row| row.id == id) {
            return Err(AutoPaperError::NotFound);
        }
        check_mood_name(&moods, Some(id), &name)?;
        tx.execute("UPDATE moods SET name = ?2 WHERE id = ?1", params![id, name])?;
        tx.commit()?;
        self.mood(id)?.ok_or(AutoPaperError::NotFound)
    }

    /// Deletes a mood and its keywords (its wallpapers stay, under no mood's name). Deleting the active mood
    /// makes the next one active (the one before when it was last). `NotFound` for an unknown id; `InvalidInput`
    /// `LastMood` for the only mood.
    pub fn delete_mood(&mut self, id: &str) -> Result<()> {
        let tx = self.write()?;
        let moods = load_moods(&tx)?;
        let Some(index) = moods.iter().position(|row| row.id == id) else {
            return Err(AutoPaperError::NotFound);
        };
        if moods.len() == 1 {
            return Err(AutoPaperError::invalid_input(InvalidInputReason::LastMood, "there's always at least one mood"));
        }
        if active_mood_id(&tx)? == id {
            let next = moods.get(index + 1).or_else(|| index.checked_sub(1).and_then(|before| moods.get(before)));
            let next = next.ok_or_else(|| internal("no mood to make active"))?;
            set_state(&tx, STATE_ACTIVE_MOOD, &Value::String(next.id.clone()))?;
        }
        tx.execute("DELETE FROM moods WHERE id = ?1", [id])?;
        let ids: Vec<String> = moods.into_iter().map(|row| row.id).filter(|other| other != id).collect();
        renumber_moods(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Makes `id` the active mood (nothing else changes). `NotFound` for an unknown id.
    pub fn set_active_mood(&mut self, id: &str) -> Result<()> {
        let tx = self.write()?;
        if !load_moods(&tx)?.iter().any(|row| row.id == id) {
            return Err(AutoPaperError::NotFound);
        }
        set_state(&tx, STATE_ACTIVE_MOOD, &Value::String(id.to_string()))?;
        tx.commit()?;
        Ok(())
    }

    /// Moves to `to_position` (clamped), renumbering the others 0..n. `NotFound` for an unknown id.
    pub fn move_mood(&mut self, id: &str, to_position: u32) -> Result<()> {
        let tx = self.write()?;
        let mut ids: Vec<String> = load_moods(&tx)?.into_iter().map(|row| row.id).collect();
        let Some(from) = ids.iter().position(|other| other == id) else {
            return Err(AutoPaperError::NotFound);
        };
        let moved = ids.remove(from);
        let to = usize::try_from(to_position).unwrap_or(usize::MAX).min(ids.len());
        ids.insert(to, moved);
        renumber_moods(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Sets one mood's Surprise (the caller clamps it). `NotFound` for an unknown id.
    pub fn set_mood_surprise(&mut self, id: &str, surprise: f32) -> Result<()> {
        found(self.conn.execute("UPDATE moods SET surprise = ?2 WHERE id = ?1", params![id, f64::from(surprise)])?)
    }

    // ── Keywords ────────────────────────────────────────────────────────────────────────────

    /// The active mood's keywords, ordered by `position` (always 0..n without gaps).
    pub fn keywords(&self) -> Result<Vec<Keyword>> {
        load_keywords(&self.conn, &active_mood_id(&self.conn)?)
    }

    /// One mood's keywords in order. `NotFound` for an unknown mood.
    pub fn mood_keywords(&self, mood_id: &str) -> Result<Vec<Keyword>> {
        if !load_moods(&self.conn)?.iter().any(|row| row.id == mood_id) {
            return Err(AutoPaperError::NotFound);
        }
        load_keywords(&self.conn, mood_id)
    }

    /// `upsert_keyword_in` the active mood.
    pub fn upsert_keyword(&mut self, text: &str, weight: KeywordWeight, now: i64) -> Result<Keyword> {
        let mood_id = active_mood_id(&self.conn)?;
        self.upsert_keyword_in(&mood_id, text, weight, now)
    }

    /// Inserts `text` (already normalised; normalising again is a no-op, and invalid text fails with
    /// `InvalidInput`) at the end of the mood's keywords, or — if a keyword of that mood with the same text
    /// (case-insensitive, Unicode-aware) exists — updates that keyword's weight and returns it, keeping its text,
    /// id and position. Fails with `InvalidInput` beyond `Keyword::MAX_COUNT` keywords in the mood; `NotFound`
    /// for an unknown mood.
    pub fn upsert_keyword_in(&mut self, mood_id: &str, text: &str, weight: KeywordWeight, now: i64) -> Result<Keyword> {
        let text = normalize_keyword(text)?;
        let tx = self.write()?;
        if !load_moods(&tx)?.iter().any(|row| row.id == mood_id) {
            return Err(AutoPaperError::NotFound);
        }
        let keywords = load_keywords(&tx, mood_id)?;
        let keyword = match keywords.iter().find(|keyword| same_text(&keyword.text, &text)) {
            Some(existing) => {
                tx.execute("UPDATE keywords SET weight = ?2 WHERE id = ?1", params![existing.id, weight])?;
                Keyword { weight, ..existing.clone() }
            }
            None => {
                if keywords.len() >= Keyword::MAX_COUNT {
                    return Err(AutoPaperError::invalid_input(
                        InvalidInputReason::TooManyKeywords,
                        format!("a mood can have at most {} keywords", Keyword::MAX_COUNT),
                    ));
                }
                let keyword = Keyword {
                    id: Uuid::now_v7().to_string(),
                    text,
                    weight,
                    position: position(keywords.len())?,
                    created_at: now,
                };
                insert_keyword(&tx, mood_id, &keyword)?;
                keyword
            }
        };
        tx.commit()?;
        Ok(keyword)
    }

    /// Any mood's keyword, by id. `NotFound` for an unknown id.
    pub fn set_keyword_weight(&mut self, id: &str, weight: KeywordWeight) -> Result<()> {
        found(self.conn.execute("UPDATE keywords SET weight = ?2 WHERE id = ?1", params![id, weight])?)
    }

    /// Any mood's keyword, by id. `NotFound` for an unknown id; `InvalidInput` if another keyword of its mood
    /// already has `text` (case-insensitive) or `text` isn't a valid keyword. Changing only the case of its own
    /// text is fine.
    pub fn rename_keyword(&mut self, id: &str, text: &str) -> Result<Keyword> {
        let text = normalize_keyword(text)?;
        let tx = self.write()?;
        let keywords = load_keywords(&tx, &keyword_mood(&tx, id)?)?;
        let Some(keyword) = keywords.iter().find(|keyword| keyword.id == id) else {
            return Err(AutoPaperError::NotFound);
        };
        if keywords.iter().any(|other| other.id != id && same_text(&other.text, &text)) {
            return Err(AutoPaperError::invalid_input(
                InvalidInputReason::DuplicateKeyword,
                "another keyword already has this text",
            ));
        }
        tx.execute("UPDATE keywords SET text = ?2 WHERE id = ?1", params![id, text])?;
        let renamed = Keyword { text, ..keyword.clone() };
        tx.commit()?;
        Ok(renamed)
    }

    /// Moves any mood's keyword to `to_position` (clamped) within its mood, renumbering the others 0..n without
    /// gaps. `NotFound` for an unknown id.
    pub fn move_keyword(&mut self, id: &str, to_position: u32) -> Result<()> {
        let tx = self.write()?;
        let mut ids: Vec<String> =
            load_keywords(&tx, &keyword_mood(&tx, id)?)?.into_iter().map(|keyword| keyword.id).collect();
        let Some(from) = ids.iter().position(|other| other == id) else {
            return Err(AutoPaperError::NotFound);
        };
        let moved = ids.remove(from);
        let to = usize::try_from(to_position).unwrap_or(usize::MAX).min(ids.len());
        ids.insert(to, moved);
        renumber(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Removes any mood's keyword and renumbers its mood. `NotFound` for an unknown id.
    pub fn delete_keyword(&mut self, id: &str) -> Result<()> {
        let tx = self.write()?;
        let mood_id = keyword_mood(&tx, id)?;
        found(tx.execute("DELETE FROM keywords WHERE id = ?1", [id])?)?;
        let ids: Vec<String> = load_keywords(&tx, &mood_id)?.into_iter().map(|keyword| keyword.id).collect();
        renumber(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    // ── Settings and engine state ───────────────────────────────────────────────────────────

    /// Stored settings, with `Settings::default()` for anything never saved — field by field, so a
    /// missing or unreadable value (say, a choice added by a newer build) only resets that one field.
    /// `surprise` is the active mood's.
    pub fn settings(&self) -> Result<Settings> {
        let mut statement = self.conn.prepare_cached("SELECT key, value FROM settings")?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<(String, String)>>>()?;
        let mut settings: Settings = decode_fields(rows)?;
        let active = active_mood_id(&self.conn)?;
        let surprise: Option<f64> =
            self.conn.query_row("SELECT surprise FROM moods WHERE id = ?1", [&active], |row| row.get(0)).optional()?;
        if let Some(surprise) = surprise {
            settings.surprise = surprise as f32;
        }
        Ok(settings)
    }

    /// Writes every field as its own row, and `surprise` into the active mood too. Rows for keys this build
    /// doesn't know (a newer build's settings) are left as they are.
    pub fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        let fields = encode_fields(settings)?;
        let tx = self.write()?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?;
            for (key, value) in &fields {
                statement.execute(params![key, value])?;
            }
        }
        let active = active_mood_id(&tx)?;
        tx.execute("UPDATE moods SET surprise = ?2 WHERE id = ?1", params![active, f64::from(settings.surprise)])?;
        tx.commit()?;
        Ok(())
    }

    /// Small engine state values (JSON), e.g. "backoff_until", "consecutive_failures".
    pub fn state_get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let text: Option<String> =
            self.conn.query_row("SELECT value FROM state WHERE key = ?1", [key], |row| row.get(0)).optional()?;
        text.map(|text| serde_json::from_str(&text).map_err(|error| storage(format!("state {key:?}: {error}"))))
            .transpose()
    }

    /// Inserts or replaces the value for `key`.
    pub fn state_set(&mut self, key: &str, value: &serde_json::Value) -> Result<()> {
        set_state(&self.conn, key, value)
    }

    // ── Generations (memory) ────────────────────────────────────────────────────────────────

    /// `InvalidInput` if a generation with the same id exists, or the cost doesn't fit the column
    /// (above `i64::MAX` micro-dollars).
    pub fn insert_generation(&mut self, stored: &StoredGeneration) -> Result<()> {
        let generation = &stored.generation;
        let concept_json = to_json(&generation.concept)?;
        let keywords_json = to_json(&generation.keywords)?;
        let cost = to_i64(generation.cost_microusd, "cost")?;
        let rated_at = (generation.rating != Rating::Unrated).then_some(generation.created_at);
        let inserted = self.conn.execute(
            INSERT_GENERATION,
            named_params! {
                ":id": generation.id,
                ":created_at": generation.created_at,
                ":trigger": generation.trigger,
                ":status": generation.status,
                ":title": generation.concept.title,
                ":summary": generation.concept.summary,
                ":concept_json": concept_json,
                ":prompt": generation.concept.prompt,
                ":text_provider": generation.text_provider,
                ":text_model": generation.text_model,
                ":image_provider": generation.image_provider,
                ":image_model": generation.image_model,
                ":width": generation.width,
                ":height": generation.height,
                ":image_path": generation.image_path,
                ":thumb_path": generation.thumb_path,
                ":phash": stored.phash.map(u64::cast_signed),
                ":embedding": embedding_blob(&stored.embedding),
                ":embedding_model": stored.embedding_model,
                ":echo_of": generation.echo_of,
                ":echo_note": generation.echo_note,
                ":surprise": generation.surprise,
                ":keywords_json": keywords_json,
                ":rating": generation.rating,
                ":rated_at": rated_at,
                ":last_shown_at": generation.last_shown_at,
                ":shown_count": generation.shown_count,
                ":cost_microusd": cost,
                ":error": generation.error,
                ":least_similar": stored.least_similar,
                ":mood_id": generation.mood_id,
            },
        );
        match inserted {
            Ok(_) => Ok(()),
            Err(error) if is_duplicate_key(&error) => Err(invalid("a generation with this id already exists")),
            Err(error) => Err(error.into()),
        }
    }

    /// Any status. `None` for an unknown id.
    pub fn generation(&self, id: &str) -> Result<Option<StoredGeneration>> {
        Ok(self
            .conn
            .prepare_cached(concat!(
                "SELECT ",
                generation_columns!(),
                ", embedding, embedding_model, phash, least_similar FROM generations WHERE id = ?1"
            ))?
            .query_row([id], stored_from_row)
            .optional()?)
    }

    /// Newest first. `Liked`/`Disliked` filter on rating; `Echoes` = generations with `echo_of` set.
    /// Only status `Ok` generations appear in history, and not ones cleared from it (`clear_history(true)`).
    /// Pruned ones (no image) are still listed.
    pub fn history(&self, filter: HistoryFilter, limit: u32, offset: u32) -> Result<Vec<Generation>> {
        let sql = match filter {
            HistoryFilter::All => history_query!(""),
            HistoryFilter::Liked => history_query!(" AND rating = 1"),
            HistoryFilter::Disliked => history_query!(" AND rating = -1"),
            HistoryFilter::Echoes => history_query!(" AND echo_of IS NOT NULL"),
        };
        query_generations(&self.conn, sql, params![limit, offset])
    }

    /// `history` of the wallpapers made under one mood (`None`: every mood, as `history`).
    pub fn history_by_mood(&self, filter: HistoryFilter, mood_id: Option<&str>, limit: u32, offset: u32) -> Result<Vec<Generation>> {
        let Some(mood_id) = mood_id else { return self.history(filter, limit, offset) };
        let sql = match filter {
            HistoryFilter::All => mood_history_query!(""),
            HistoryFilter::Liked => mood_history_query!(" AND rating = 1"),
            HistoryFilter::Disliked => mood_history_query!(" AND rating = -1"),
            HistoryFilter::Echoes => mood_history_query!(" AND echo_of IS NOT NULL"),
        };
        query_generations(&self.conn, sql, params![limit, offset, mood_id])
    }

    /// One `MoodStats` per mood, in the person's order (a mood that has made nothing is there, at zero), with each
    /// mood's newest `latest` wallpapers. Counts what History lists per mood. Two queries, whatever the number of
    /// moods.
    pub fn mood_stats(&self, latest: u32) -> Result<Vec<MoodStats>> {
        let mut counts: HashMap<String, (u32, u32, u32, u32, Option<i64>)> = HashMap::new();
        let mut statement = self.conn.prepare_cached(MOOD_COUNTS)?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        })?;
        for row in rows {
            let (mood, numbers) = row?;
            counts.insert(mood, numbers);
        }
        let mut newest: HashMap<String, Vec<Generation>> = HashMap::new();
        for generation in query_generations(&self.conn, MOOD_LATEST, [latest])? {
            if let Some(mood) = generation.mood_id.clone() {
                newest.entry(mood).or_default().push(generation);
            }
        }
        Ok(load_moods(&self.conn)?
            .into_iter()
            .map(|mood| {
                let (wallpapers, liked, disliked, echoes, last_made_at) = counts.remove(&mood.id).unwrap_or_default();
                let latest = newest.remove(&mood.id).unwrap_or_default();
                MoodStats { mood_id: mood.id, wallpapers, liked, disliked, echoes, last_made_at, latest }
            })
            .collect())
    }

    /// Wallpapers per day and mood: day `i` runs from `day_bounds[i]` up to `day_bounds[i + 1]`. Counts what
    /// History lists; only days and moods with wallpapers, by day, then in the moods' order (deleted moods' last,
    /// as `None`). `InvalidInput` for fewer than 2 or more than `DayCount::MAX_DAYS + 1` bounds, or bounds that
    /// don't ascend.
    pub fn activity(&self, day_bounds: &[i64]) -> Result<Vec<DayCount>> {
        if day_bounds.len() < 2 || day_bounds.len() > DayCount::MAX_DAYS + 1 {
            return Err(invalid(format!("activity takes 2 to {} day bounds", DayCount::MAX_DAYS + 1)));
        }
        if day_bounds.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid("day bounds must ascend"));
        }
        let order: HashMap<String, usize> =
            load_moods(&self.conn)?.into_iter().enumerate().map(|(index, mood)| (mood.id, index)).collect();
        let mut statement = self.conn.prepare_cached(
            "SELECT created_at, mood_id FROM generations
             WHERE status = 'ok' AND hidden = 0 AND created_at >= ?1 AND created_at < ?2",
        )?;
        let rows = statement.query_map(params![day_bounds[0], day_bounds[day_bounds.len() - 1]], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        // (day, the mood's place in the list, or after every mood) → (mood, count).
        let mut days: BTreeMap<(usize, usize), (Option<String>, u32)> = BTreeMap::new();
        for row in rows {
            let (made_at, mood) = row?;
            let day = day_bounds.partition_point(|start| *start <= made_at) - 1;
            let mood = mood.filter(|id| order.contains_key(id));
            let place = mood.as_ref().map_or(usize::MAX, |id| order[id]);
            days.entry((day, place)).or_insert((mood, 0)).1 += 1;
        }
        Ok(days
            .into_iter()
            .map(|((day, _), (mood_id, count))| DayCount { day_start: day_bounds[day], mood_id, count })
            .collect())
    }

    /// The generation with the latest `last_shown_at` (the later inserted one on a tie), not counting ones
    /// cleared from history; `None` before anything has been shown.
    pub fn current(&self) -> Result<Option<Generation>> {
        Ok(self
            .conn
            .prepare_cached(concat!(
                "SELECT ",
                generation_columns!(),
                " FROM generations WHERE last_shown_at IS NOT NULL AND hidden = 0
                 ORDER BY last_shown_at DESC, rowid DESC LIMIT 1"
            ))?
            .query_row([], generation_from_row)
            .optional()?)
    }

    /// Sets `last_shown_at = now`, increments `shown_count`. `NotFound` for an unknown id.
    pub fn mark_shown(&mut self, id: &str, now: i64) -> Result<()> {
        found(self.conn.execute(
            "UPDATE generations SET last_shown_at = ?2, shown_count = shown_count + 1 WHERE id = ?1",
            params![id, now],
        )?)
    }

    /// Sets the rating; returns the previous one (taste needs both). `NotFound` for an unknown id.
    /// Records `now` as the rating time (cleared when Unrated); the same rating again changes nothing.
    pub fn set_rating(&mut self, id: &str, rating: Rating, now: i64) -> Result<Rating> {
        let tx = self.write()?;
        let previous: Rating = tx
            .query_row("SELECT rating FROM generations WHERE id = ?1", [id], |row| row.get(0))
            .optional()?
            .ok_or(AutoPaperError::NotFound)?;
        if previous != rating {
            let rated_at = (rating != Rating::Unrated).then_some(now);
            tx.execute(
                "UPDATE generations SET rating = ?2, rated_at = ?3 WHERE id = ?1",
                params![id, rating, rated_at],
            )?;
        }
        tx.commit()?;
        Ok(previous)
    }

    /// Every successful generation's memory, oldest first (novelty and echoes scan all of it).
    pub fn memory(&self) -> Result<Vec<MemoryRow>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, created_at, title, summary, embedding, embedding_model, rating, echo_of, mood_id
             FROM generations WHERE status = 'ok' ORDER BY created_at, rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(MemoryRow {
                id: row.get("id")?,
                created_at: row.get("created_at")?,
                title: row.get("title")?,
                summary: row.get("summary")?,
                embedding: row.get::<_, EmbeddingBlob>("embedding")?.0,
                embedding_model: row.get("embedding_model")?,
                rating: row.get("rating")?,
                echo_of: row.get("echo_of")?,
                mood_id: row.get("mood_id")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces stored embeddings (after an embedding-model change). `NotFound` for an unknown id.
    pub fn update_embedding(&mut self, id: &str, embedding: &[f32], model: &str) -> Result<()> {
        found(self.conn.execute(
            "UPDATE generations SET embedding = ?2, embedding_model = ?3 WHERE id = ?1",
            params![id, embedding_blob(embedding), model],
        )?)
    }

    /// Summaries of the `n` most recent successful generations, newest first.
    pub fn recent_summaries(&self, n: u32) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT summary FROM generations WHERE status = 'ok' ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([n], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The root original and every generation descending from it via `echo_of`, oldest first;
    /// successful generations only, not counting ones cleared from history. A deleted (or cleared) original
    /// still ties its echoes together (its id is kept in their `echo_of`), it just isn't listed. `NotFound`
    /// for an unknown id.
    pub fn lineage(&self, id: &str) -> Result<Vec<Generation>> {
        // One read transaction, so the walk up and the walk down see the same snapshot.
        let tx = self.conn.unchecked_transaction()?;
        let root = lineage_root(&tx, id)?;
        let lineage = query_generations(
            &tx,
            concat!(
                "WITH RECURSIVE lineage (id) AS (
                     SELECT ?1
                     UNION
                     SELECT generations.id FROM generations JOIN lineage ON generations.echo_of = lineage.id
                 )
                 SELECT ",
                generation_columns!(),
                " FROM generations WHERE id IN (SELECT id FROM lineage) AND status = 'ok' AND hidden = 0
                 ORDER BY created_at, rowid"
            ),
            [root],
        )?;
        tx.commit()?;
        Ok(lineage)
    }

    /// Whether `lineage(id)` lists any generation besides `id` itself: its original or another echo in the same
    /// lineage, still in history (status ok, not cleared). One query, without loading the generations.
    /// `NotFound` for an unknown id.
    pub fn has_relatives(&self, id: &str) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let root = lineage_root(&tx, id)?;
        let found = tx.query_row(
            "WITH RECURSIVE lineage (id) AS (
                 SELECT ?1
                 UNION
                 SELECT generations.id FROM generations JOIN lineage ON generations.echo_of = lineage.id
             )
             SELECT EXISTS (
                 SELECT 1 FROM generations
                 WHERE id IN (SELECT id FROM lineage) AND id <> ?2 AND status = 'ok' AND hidden = 0
             )",
            params![root, id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(found)
    }

    /// Created-at of the newest successful generation, whatever its trigger (scheduled, manual, a disliked
    /// one's replacement, an echo; cleared history included). The schedule counts from the newest new
    /// wallpaper; the engine records that as it makes each one, and reads this only from a database that has
    /// no record yet.
    pub fn last_new_at(&self) -> Result<Option<i64>> {
        Ok(self.conn.query_row("SELECT MAX(created_at) FROM generations WHERE status = 'ok'", [], |row| row.get(0))?)
    }

    /// For each of the `limit` newest successful generations that aren't echoes (any trigger, cleared
    /// history included) — made under `mood_id`, or under any mood — whether it took the least similar
    /// candidate; newest first.
    pub fn recent_least_similar(&self, limit: u32, mood_id: Option<&str>) -> Result<Vec<bool>> {
        let rows = match mood_id {
            Some(mood_id) => {
                let mut statement = self.conn.prepare_cached(
                    "SELECT least_similar FROM generations WHERE mood_id = ?2 AND status = 'ok' AND echo_of IS NULL
                     ORDER BY created_at DESC, rowid DESC LIMIT ?1",
                )?;
                statement.query_map(params![limit, mood_id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
            }
            None => {
                let mut statement = self.conn.prepare_cached(
                    "SELECT least_similar FROM generations WHERE status = 'ok' AND echo_of IS NULL
                     ORDER BY created_at DESC, rowid DESC LIMIT ?1",
                )?;
                statement.query_map([limit], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
            }
        };
        Ok(rows)
    }

    /// Deletes failed and refused attempts made before `cutoff` (Unix seconds); returns how many. They have
    /// no files and are in neither history nor memory.
    pub fn delete_unsuccessful_before(&mut self, cutoff: i64) -> Result<u32> {
        let deleted = self.conn.execute(
            "DELETE FROM generations WHERE status IN ('failed', 'refused') AND created_at < ?1",
            [cutoff],
        )?;
        Ok(u32::try_from(deleted).unwrap_or(u32::MAX))
    }

    /// Liked, status ok, image path still set; least recently shown first (never shown before any).
    /// The engine checks that the files still exist.
    pub fn liked_with_images(&self) -> Result<Vec<Generation>> {
        query_generations(
            &self.conn,
            concat!(
                "SELECT ",
                generation_columns!(),
                " FROM generations WHERE rating = 1 AND status = 'ok' AND hidden = 0 AND image_path IS NOT NULL
                 ORDER BY last_shown_at ASC NULLS FIRST, created_at, rowid"
            ),
            [],
        )
    }

    /// Sets or clears (on prune) the image and thumbnail paths. `NotFound` for an unknown id.
    pub fn set_image_paths(&mut self, id: &str, image: Option<&str>, thumb: Option<&str>) -> Result<()> {
        found(self.conn.execute(
            "UPDATE generations SET image_path = ?2, thumb_path = ?3 WHERE id = ?1",
            params![id, image, thumb],
        )?)
    }

    /// Generations with an image on disk, as prune candidates: oldest first, excluding liked ones.
    pub fn prunable(&self) -> Result<Vec<Generation>> {
        query_generations(
            &self.conn,
            concat!(
                "SELECT ",
                generation_columns!(),
                " FROM generations WHERE image_path IS NOT NULL AND rating <> 1 ORDER BY created_at, rowid"
            ),
            [],
        )
    }

    /// Removes one generation entirely; echoes of it keep their `echo_of` id (it's history).
    /// `NotFound` for an unknown id.
    pub fn delete_generation(&mut self, id: &str) -> Result<()> {
        found(self.conn.execute("DELETE FROM generations WHERE id = ?1", [id])?)
    }

    /// With `keep_memory`: keeps every row (concepts, embeddings, ratings), hides it from history,
    /// `current` and `lineage`, and clears image and thumbnail paths (the engine deletes the files).
    /// Without: deletes all generations and taste. Keywords, settings, engine state and spend (money
    /// already spent this month still counts against the budget) stay either way.
    pub fn clear_history(&mut self, keep_memory: bool) -> Result<()> {
        let tx = self.write()?;
        if keep_memory {
            tx.execute(
                "UPDATE generations SET image_path = NULL, thumb_path = NULL, hidden = 1
                 WHERE image_path IS NOT NULL OR thumb_path IS NOT NULL OR hidden = 0",
                [],
            )?;
        } else {
            tx.execute_batch("DELETE FROM generations; DELETE FROM taste;")?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Ids of generations cleared from history but kept in memory (`clear_history(true)`); echoes skip them.
    pub fn hidden_ids(&self) -> Result<HashSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT id FROM generations WHERE hidden = 1")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The id of every generation, any status (image files named after anything else have no owner).
    pub fn generation_ids(&self) -> Result<HashSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT id FROM generations")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Concepts of the `limit` newest successful generations with this rating, including ones cleared from
    /// history (their taste is kept, so the words for it are too).
    pub fn rated_concepts(&self, rating: Rating, limit: u32) -> Result<Vec<Concept>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT concept_json FROM generations WHERE rating = ?1 AND status = 'ok'
             ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![rating, limit], |row| Ok(row.get::<_, ConceptJson>(0)?.0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Number of successful generations (what memory holds, cleared history included).
    pub fn generation_count(&self) -> Result<u32> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM generations WHERE status = 'ok'", [], |row| row.get(0))?)
    }

    // ── Taste ───────────────────────────────────────────────────────────────────────────────

    /// Ordered by feature.
    pub fn taste_rows(&self) -> Result<Vec<TasteRow>> {
        let mut statement =
            self.conn.prepare_cached("SELECT feature, likes, dislikes, updated_at FROM taste ORDER BY feature")?;
        let rows = statement.query_map([], |row| {
            Ok(TasteRow { feature: row.get(0)?, likes: row.get(1)?, dislikes: row.get(2)?, updated_at: row.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Inserts or replaces each row by feature.
    pub fn save_taste_rows(&mut self, rows: &[TasteRow]) -> Result<()> {
        let tx = self.write()?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO taste (feature, likes, dislikes, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (feature) DO UPDATE SET
                     likes = excluded.likes, dislikes = excluded.dislikes, updated_at = excluded.updated_at",
            )?;
            for row in rows {
                statement.execute(params![row.feature, row.likes, row.dislikes, row.updated_at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reset_taste(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM taste", [])?;
        Ok(())
    }

    /// Number of generations rated Liked or Disliked.
    pub fn rated_count(&self) -> Result<u32> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM generations WHERE rating <> 0", [], |row| row.get(0))?)
    }

    // ── Timings ─────────────────────────────────────────────────────────────────────────────

    /// Records one finished provider call, keeping the newest `TIMINGS_KEPT` of its kind (same job, provider,
    /// origin and model).
    pub fn add_timing(&mut self, timing: &Timing) -> Result<()> {
        let tx = self.write()?;
        tx.execute(
            "INSERT INTO timings (job, provider, origin, model, width, height, steps, seconds, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                timing.job,
                timing.provider,
                timing.origin,
                timing.model,
                timing.width,
                timing.height,
                timing.steps,
                timing.seconds,
                timing.finished_at
            ],
        )?;
        tx.execute(
            "DELETE FROM timings WHERE job = ?1 AND provider = ?2 AND origin = ?3 AND model = ?4 AND id NOT IN (
                 SELECT id FROM timings WHERE job = ?1 AND provider = ?2 AND origin = ?3 AND model = ?4
                 ORDER BY finished_at DESC, id DESC LIMIT ?5
             )",
            params![timing.job, timing.provider, timing.origin, timing.model, TIMINGS_KEPT],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The recorded calls of one kind (job, provider, origin, model), newest first.
    pub fn timings(&self, job: ProviderJob, provider: ProviderKind, origin: &str, model: &str) -> Result<Vec<Timing>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT job, provider, origin, model, width, height, steps, seconds, finished_at FROM timings
             WHERE job = ?1 AND provider = ?2 AND origin = ?3 AND model = ?4
             ORDER BY finished_at DESC, id DESC LIMIT ?5",
        )?;
        let rows = statement.query_map(params![job, provider, origin, model, TIMINGS_KEPT], |row| {
            Ok(Timing {
                job: row.get(0)?,
                provider: row.get(1)?,
                origin: row.get(2)?,
                model: row.get(3)?,
                width: row.get(4)?,
                height: row.get(5)?,
                steps: row.get(6)?,
                seconds: row.get(7)?,
                finished_at: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ── Spend ───────────────────────────────────────────────────────────────────────────────

    /// Adds to the month's totals ("YYYY-MM"; anything else is `InvalidInput`).
    pub fn add_spend(&mut self, month: &str, microusd: u64, images: u32) -> Result<()> {
        check_month(month)?;
        let microusd = to_i64(microusd, "spend")?;
        self.conn.execute(
            "INSERT INTO spend (month, microusd, images) VALUES (?1, ?2, ?3)
             ON CONFLICT (month) DO UPDATE SET
                 microusd = microusd + excluded.microusd, images = images + excluded.images",
            params![month, microusd, images],
        )?;
        Ok(())
    }

    /// (micro-USD spent, images made) for the month; zeros when none. `InvalidInput` unless "YYYY-MM".
    pub fn spend(&self, month: &str) -> Result<(u64, u32)> {
        check_month(month)?;
        let totals = self
            .conn
            .query_row("SELECT microusd, images FROM spend WHERE month = ?1", [month], |row| {
                Ok((row.get::<_, Unsigned>(0)?.0, row.get(1)?))
            })
            .optional()?;
        Ok(totals.unwrap_or((0, 0)))
    }
}

// ── Migrations ──────────────────────────────────────────────────────────────────────────────

/// Applies the migrations `user_version` says are missing, each in its own IMMEDIATE transaction
/// together with its `user_version` bump (a failure leaves the previous version intact). The version
/// is re-read inside each transaction, so two processes opening a new file don't migrate it twice.
fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<()> {
    let latest = migrations.len();
    let current = schema_version(conn)?;
    if current >= latest {
        if current > latest {
            tracing::warn!(current, latest, "database is from a newer AutoPaper; opening it without migrating");
        }
        return Ok(());
    }
    for (index, migration) in migrations.iter().enumerate().skip(current) {
        let version = index + 1;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if schema_version(&tx)? >= version {
            continue;
        }
        tx.execute_batch(migration.sql)?;
        if let Some(data) = migration.data {
            data(&tx)?;
        }
        let stamp = i64::try_from(version).map_err(|_| internal("schema version out of range"))?;
        tx.pragma_update(None, "user_version", stamp)?;
        tx.commit()?;
    }
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<usize> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    usize::try_from(version).map_err(|_| storage(format!("unexpected schema version {version}")))
}

/// Migration 3's data step: the keywords and Settings' Surprise become the first mood, named from its first two
/// keywords that aren't Avoids ("Rain, Beach"), else "My mood"; every keyword and every wallpaper so far belongs to
/// it, and it is active.
fn first_mood(conn: &Connection) -> Result<()> {
    let keywords: Vec<(String, String)> = {
        let mut statement = conn.prepare("SELECT text, weight FROM keywords ORDER BY position, rowid")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // A weight this build doesn't know (a newer build's) only matters for naming: it isn't an Avoid.
    let named = keywords.iter().map(|(text, weight)| {
        (text.as_str(), if weight == "avoid" { KeywordWeight::Avoid } else { KeywordWeight::Must })
    });
    let name = text::mood_name_from_keywords(named);
    let stored: Option<String> =
        conn.query_row("SELECT value FROM settings WHERE key = 'surprise'", [], |row| row.get(0)).optional()?;
    let surprise = stored
        .and_then(|value| serde_json::from_str::<f64>(&value).ok())
        .filter(|surprise| surprise.is_finite())
        .map_or(Settings::default().surprise, |surprise| surprise.clamp(0.0, 1.0) as f32);
    let created_at: Option<i64> = conn.query_row("SELECT MIN(created_at) FROM keywords", [], |row| row.get(0))?;
    let id = insert_mood(conn, &name, 0, surprise, created_at.unwrap_or_else(unix_now))?;
    conn.execute("UPDATE keywords SET mood_id = ?1", [&id])?;
    conn.execute("UPDATE generations SET mood_id = ?1", [&id])?;
    set_state(conn, STATE_ACTIVE_MOOD, &Value::String(id))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

fn set_state(conn: &Connection, key: &str, value: &Value) -> Result<()> {
    conn.execute(
        "INSERT INTO state (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

// ── Moods ───────────────────────────────────────────────────────────────────────────────────

/// A `moods` row.
struct MoodRow {
    id: String,
    name: String,
    position: u32,
    surprise: f32,
    created_at: i64,
}

impl MoodRow {
    fn into_mood(self, conn: &Connection, active: &str) -> Result<Mood> {
        let keywords = load_keywords(conn, &self.id)?;
        Ok(Mood {
            active: self.id == active,
            id: self.id,
            name: self.name,
            position: self.position,
            surprise: self.surprise,
            created_at: self.created_at,
            keywords,
        })
    }
}

fn load_moods(conn: &Connection) -> Result<Vec<MoodRow>> {
    let mut statement =
        conn.prepare_cached("SELECT id, name, position, surprise, created_at FROM moods ORDER BY position, rowid")?;
    let rows = statement.query_map([], |row| {
        Ok(MoodRow {
            id: row.get(0)?,
            name: row.get(1)?,
            position: row.get(2)?,
            surprise: row.get::<_, f64>(3)? as f32,
            created_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The id named in state when that mood exists, else the first mood's. `Internal` with no moods at all (the
/// store makes one when it opens).
fn active_mood_id(conn: &Connection) -> Result<String> {
    let named: Option<String> =
        conn.query_row("SELECT value FROM state WHERE key = ?1", [STATE_ACTIVE_MOOD], |row| row.get(0)).optional()?;
    if let Some(Value::String(id)) = named.and_then(|text| serde_json::from_str(&text).ok()) {
        let exists: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM moods WHERE id = ?1)", [&id], |row| row.get(0))?;
        if exists {
            return Ok(id);
        }
    }
    conn.query_row("SELECT id FROM moods ORDER BY position, rowid LIMIT 1", [], |row| row.get(0))
        .optional()?
        .ok_or_else(|| internal("there is no mood"))
}

/// Inserts a mood (name already normalised and checked); returns its new id.
fn insert_mood(conn: &Connection, name: &str, position: u32, surprise: f32, now: i64) -> Result<String> {
    let id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO moods (id, name, position, surprise, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, position, f64::from(surprise), now],
    )?;
    Ok(id)
}

/// `DuplicateMoodName` when a mood other than `except` already has `name` (case-insensitive, Unicode-aware).
fn check_mood_name(moods: &[MoodRow], except: Option<&str>, name: &str) -> Result<()> {
    if moods.iter().any(|row| Some(row.id.as_str()) != except && same_text(&row.name, name)) {
        return Err(AutoPaperError::invalid_input(InvalidInputReason::DuplicateMoodName, "another mood has this name"));
    }
    Ok(())
}

/// Gives the moods `ids` positions 0..n in order, writing only the rows that change.
fn renumber_moods(conn: &Connection, ids: &[String]) -> Result<()> {
    let mut statement = conn.prepare_cached("UPDATE moods SET position = ?2 WHERE id = ?1 AND position <> ?2")?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![id, position(index)?])?;
    }
    Ok(())
}

// ── Keywords ────────────────────────────────────────────────────────────────────────────────

/// The mood a keyword belongs to. `NotFound` for an unknown keyword.
fn keyword_mood(conn: &Connection, id: &str) -> Result<String> {
    let mood: Option<Option<String>> =
        conn.query_row("SELECT mood_id FROM keywords WHERE id = ?1", [id], |row| row.get(0)).optional()?;
    match mood {
        Some(Some(mood)) => Ok(mood),
        Some(None) => active_mood_id(conn),
        None => Err(AutoPaperError::NotFound),
    }
}

fn insert_keyword(conn: &Connection, mood_id: &str, keyword: &Keyword) -> Result<()> {
    conn.execute(
        "INSERT INTO keywords (id, mood_id, text, weight, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![keyword.id, mood_id, keyword.text, keyword.weight, keyword.position, keyword.created_at],
    )?;
    Ok(())
}

/// One mood's keywords in order.
fn load_keywords(conn: &Connection, mood_id: &str) -> Result<Vec<Keyword>> {
    let mut statement = conn.prepare_cached(
        "SELECT id, text, weight, position, created_at FROM keywords WHERE mood_id = ?1 ORDER BY position, rowid",
    )?;
    let rows = statement.query_map([mood_id], |row| {
        Ok(Keyword {
            id: row.get(0)?,
            text: row.get(1)?,
            weight: row.get(2)?,
            position: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Gives `ids` positions 0..n in order, writing only the rows that change.
fn renumber(conn: &Connection, ids: &[String]) -> Result<()> {
    let mut statement = conn.prepare_cached("UPDATE keywords SET position = ?2 WHERE id = ?1 AND position <> ?2")?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![id, position(index)?])?;
    }
    Ok(())
}

fn position(index: usize) -> Result<u32> {
    u32::try_from(index).map_err(|_| internal("keyword position out of range"))
}

/// Case-insensitive comparison that covers all of Unicode (SQLite's NOCASE folds only ASCII, so the
/// column's UNIQUE constraint is the backstop and this is the rule).
fn same_text(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

// ── Settings ────────────────────────────────────────────────────────────────────────────────

/// One `(key, JSON)` pair per top-level field of `value`.
fn encode_fields<T: Serialize>(value: &T) -> Result<Vec<(String, String)>> {
    match serde_json::to_value(value).map_err(|error| internal(error.to_string()))? {
        Value::Object(fields) => Ok(fields.into_iter().map(|(key, value)| (key, value.to_string())).collect()),
        _ => Err(internal("settings must serialise to a JSON object")),
    }
}

/// Rebuilds a `T` from `(key, JSON)` rows on top of `T::default()`. Keys `T` doesn't have are ignored;
/// a value that isn't JSON or doesn't fit its field is logged (key only) and its default kept.
fn decode_fields<T: Default + Serialize + DeserializeOwned>(rows: Vec<(String, String)>) -> Result<T> {
    let Value::Object(defaults) = serde_json::to_value(T::default()).map_err(|error| internal(error.to_string()))?
    else {
        return Err(internal("settings must serialise to a JSON object"));
    };
    let mut merged = defaults.clone();
    for (key, text) in rows {
        if !defaults.contains_key(&key) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            tracing::warn!(%key, "stored setting isn't JSON; using the default");
            continue;
        };
        let mut candidate = defaults.clone();
        candidate.insert(key.clone(), value.clone());
        if serde_json::from_value::<T>(Value::Object(candidate)).is_ok() {
            merged.insert(key, value);
        } else {
            tracing::warn!(%key, "stored setting doesn't fit this build; using the default");
        }
    }
    serde_json::from_value(Value::Object(merged)).map_err(|error| storage(format!("settings: {error}")))
}

// ── Generations ─────────────────────────────────────────────────────────────────────────────

fn query_generations<P: Params>(conn: &Connection, sql: &str, params: P) -> Result<Vec<Generation>> {
    let mut statement = conn.prepare_cached(sql)?;
    let rows = statement.query_map(params, generation_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Reads the columns listed in `generation_columns!`.
fn generation_from_row(row: &Row<'_>) -> rusqlite::Result<Generation> {
    Ok(Generation {
        id: row.get("id")?,
        created_at: row.get("created_at")?,
        trigger: row.get("trigger")?,
        status: row.get("status")?,
        concept: row.get::<_, ConceptJson>("concept_json")?.0,
        image_path: row.get("image_path")?,
        thumb_path: row.get("thumb_path")?,
        width: row.get("width")?,
        height: row.get("height")?,
        rating: row.get("rating")?,
        echo_of: row.get("echo_of")?,
        echo_note: row.get("echo_note")?,
        text_provider: row.get("text_provider")?,
        text_model: row.get("text_model")?,
        image_provider: row.get("image_provider")?,
        image_model: row.get("image_model")?,
        surprise: row.get("surprise")?,
        keywords: row.get::<_, Json<Vec<KeywordSnapshot>>>("keywords_json")?.0,
        cost_microusd: row.get::<_, Unsigned>("cost_microusd")?.0,
        last_shown_at: row.get("last_shown_at")?,
        shown_count: row.get("shown_count")?,
        error: row.get("error")?,
        mood_id: row.get("mood_id")?,
        mood_name: row.get("mood_name")?,
    })
}

fn stored_from_row(row: &Row<'_>) -> rusqlite::Result<StoredGeneration> {
    Ok(StoredGeneration {
        generation: generation_from_row(row)?,
        embedding: row.get::<_, EmbeddingBlob>("embedding")?.0,
        embedding_model: row.get("embedding_model")?,
        phash: row.get::<_, Option<i64>>("phash")?.map(i64::cast_unsigned),
        least_similar: row.get("least_similar")?,
    })
}

/// Follows `echo_of` up from `id` to the lineage's root. The root may have been deleted: its id still
/// names the lineage. Stops where a (corrupt) cycle would close.
fn lineage_root(conn: &Connection, id: &str) -> Result<String> {
    let mut parent_of = conn.prepare_cached("SELECT echo_of FROM generations WHERE id = ?1")?;
    let Some(mut parent) = parent_of.query_row([id], |row| row.get::<_, Option<String>>(0)).optional()? else {
        return Err(AutoPaperError::NotFound);
    };
    let mut root = id.to_string();
    let mut seen = HashSet::from([root.clone()]);
    while let Some(next) = parent {
        if !seen.insert(next.clone()) {
            break;
        }
        parent = parent_of.query_row([&next], |row| row.get(0)).optional()?.flatten();
        root = next;
    }
    Ok(root)
}

fn embedding_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|value| value.to_le_bytes()).collect()
}

/// `InvalidInput` for SQLite's primary-key violation.
fn is_duplicate_key(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(failure, _)
        if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY)
}

// ── Spend ───────────────────────────────────────────────────────────────────────────────────

/// "YYYY-MM" with a month from 01 to 12.
fn check_month(month: &str) -> Result<()> {
    let valid = match month.as_bytes() {
        [y1, y2, y3, y4, b'-', m1, m2] => {
            [y1, y2, y3, y4, m1, m2].iter().all(|digit| digit.is_ascii_digit())
                && matches!((m1, m2), (b'0', b'1'..=b'9') | (b'1', b'0'..=b'2'))
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid(format!("expected a month as YYYY-MM, got {month:?}"))) }
}

// ── Column types ────────────────────────────────────────────────────────────────────────────

/// Stable database names for enums, spelled out (rather than derived) so renaming a Rust variant can't
/// change stored data. They equal serde's snake_case names, which settings JSON and keyword snapshots
/// use, so one spelling appears everywhere in the file (a test checks this).
macro_rules! text_enum {
    ($type:ident { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl ToSql for $type {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(match self {
                    $($type::$variant => $name,)+
                }))
            }
        }

        impl FromSql for $type {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                match value.as_str()? {
                    $($name => Ok($type::$variant),)+
                    other => Err(FromSqlError::Other(format!("unknown {} {other:?}", stringify!($type)).into())),
                }
            }
        }
    };
}

text_enum!(KeywordWeight { Must => "must", Maybe => "maybe", Avoid => "avoid" });
text_enum!(Trigger {
    Scheduled => "scheduled",
    Manual => "manual",
    DislikeReplace => "dislike_replace",
    EchoRequest => "echo_request",
});
text_enum!(GenerationStatus { Ok => "ok", Failed => "failed", Refused => "refused" });
text_enum!(ProviderJob { Concepts => "concepts", Images => "images" });
text_enum!(ProviderKind {
    OpenAi => "open_ai",
    Google => "google",
    Ollama => "ollama",
    OpenAiCompatible => "open_ai_compatible",
    ComfyUi => "comfy_ui",
    Demo => "demo",
});

/// Ratings are stored as -1, 0, 1 (a CHECK constraint keeps them there).
impl ToSql for Rating {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(i64::from(self.as_i8())))
    }
}

impl FromSql for Rating {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_i64()? {
            -1 => Ok(Rating::Disliked),
            0 => Ok(Rating::Unrated),
            1 => Ok(Rating::Liked),
            other => Err(FromSqlError::OutOfRange(other)),
        }
    }
}

/// A JSON text column.
struct Json<T>(T);

impl<T: DeserializeOwned> FromSql for Json<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        serde_json::from_str(value.as_str()?).map(Json).map_err(FromSqlError::other)
    }
}

/// A concept's JSON. Fields an older build didn't write take `Concept::default()` values; fields a
/// newer build added are ignored.
struct ConceptJson(Concept);

impl FromSql for ConceptJson {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let stored: Value = serde_json::from_str(value.as_str()?).map_err(FromSqlError::other)?;
        let Value::Object(fields) = stored else {
            return Err(FromSqlError::Other("concept isn't a JSON object".into()));
        };
        let mut merged = match serde_json::to_value(Concept::default()).map_err(FromSqlError::other)? {
            Value::Object(defaults) => defaults,
            _ => Map::new(),
        };
        merged.extend(fields);
        serde_json::from_value(Value::Object(merged)).map(ConceptJson).map_err(FromSqlError::other)
    }
}

/// Little-endian f32s in a BLOB.
struct EmbeddingBlob(Vec<f32>);

impl FromSql for EmbeddingBlob {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let bytes = value.as_blob()?;
        let (floats, rest) = bytes.as_chunks::<4>();
        if !rest.is_empty() {
            return Err(FromSqlError::Other(
                format!("embedding of {} bytes isn't a whole number of f32s", bytes.len()).into(),
            ));
        }
        Ok(EmbeddingBlob(floats.iter().map(|chunk| f32::from_le_bytes(*chunk)).collect()))
    }
}

/// A non-negative INTEGER as `u64` (rusqlite reads `u64` only with its `fallible_uint` feature).
struct Unsigned(u64);

impl FromSql for Unsigned {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let signed = value.as_i64()?;
        u64::try_from(signed).map(Unsigned).map_err(|_| FromSqlError::OutOfRange(signed))
    }
}

fn to_i64(value: u64, what: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid(format!("{what} is too large to store")))
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|error| internal(error.to_string()))
}

// ── Errors ──────────────────────────────────────────────────────────────────────────────────

/// `NotFound` when an UPDATE or DELETE by id changed nothing.
fn found(changed: usize) -> Result<()> {
    if changed == 0 { Err(AutoPaperError::NotFound) } else { Ok(()) }
}

/// `InvalidInput` for a mistake no person makes (a duplicate id, a malformed month): reason `Other`.
fn invalid(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::invalid_input(InvalidInputReason::Other, detail)
}

fn storage(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::Storage { detail: detail.into() }
}

fn internal(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::Internal { detail: detail.into() }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::model::{Cadence, EchoFrequency, Fallback, ImageQuality, ProviderSelection, QuietPeriod};

    /// 2026-10-05 00:00 UTC; tests count days from here.
    const T0: i64 = 1_791_158_400;
    const DAY: i64 = 86_400;

    fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    fn concept(title: &str) -> Concept {
        Concept {
            title: title.to_string(),
            summary: format!("{title}, as a quiet landscape."),
            setting: "coast".into(),
            subject: "lighthouse".into(),
            elements: vec!["lighthouse".into(), "fog".into()],
            time_of_day: "dusk".into(),
            weather: "fog".into(),
            season: "autumn".into(),
            mood: vec!["calm".into()],
            palette: vec!["slate".into(), "amber".into()],
            style: "photograph".into(),
            composition: "wide, horizon low".into(),
            keywords_used: vec!["fog".into()],
            wildcards: vec![],
            prompt: format!("A desktop wallpaper: {title}."),
        }
    }

    /// A successful scheduled generation `day` days after `T0`, unrated, with an image.
    fn made(id: &str, day: i64) -> StoredGeneration {
        StoredGeneration {
            generation: Generation {
                id: id.to_string(),
                created_at: T0 + day * DAY,
                trigger: Trigger::Scheduled,
                status: GenerationStatus::Ok,
                concept: concept(id),
                image_path: Some(format!("images/2026/{id}.png")),
                thumb_path: Some(format!("thumbs/{id}.jpg")),
                width: 3840,
                height: 2160,
                rating: Rating::Unrated,
                echo_of: None,
                echo_note: None,
                text_provider: ProviderKind::Demo,
                text_model: "demo".into(),
                image_provider: ProviderKind::Demo,
                image_model: "demo".into(),
                surprise: 0.35,
                keywords: vec![KeywordSnapshot { text: "fog".into(), weight: KeywordWeight::Must }],
                cost_microusd: 0,
                last_shown_at: None,
                shown_count: 0,
                error: None,
                mood_id: None,
                mood_name: None,
            },
            embedding: vec![0.6, 0.8],
            embedding_model: "test-embedder".into(),
            phash: Some(0x0123_4567_89ab_cdef),
            least_similar: false,
        }
    }

    fn with(mut stored: StoredGeneration, change: impl FnOnce(&mut Generation)) -> StoredGeneration {
        change(&mut stored.generation);
        stored
    }

    fn failed(id: &str, day: i64) -> StoredGeneration {
        let mut stored = with(made(id, day), |g| {
            g.status = GenerationStatus::Failed;
            g.image_path = None;
            g.thumb_path = None;
            g.error = Some("image provider unavailable".into());
        });
        stored.embedding.clear();
        stored.phash = None;
        stored
    }

    fn insert_all(store: &mut Store, generations: Vec<StoredGeneration>) {
        for stored in generations {
            store.insert_generation(&stored).expect("insert");
        }
    }

    fn ids(generations: &[Generation]) -> Vec<&str> {
        generations.iter().map(|g| g.id.as_str()).collect()
    }

    fn raw<T: FromSql>(store: &Store, sql: &str, id: &str) -> T {
        store.conn.query_row(sql, [id], |row| row.get(0)).expect("raw query")
    }

    fn keyword_texts(store: &Store) -> Vec<String> {
        store.keywords().expect("keywords").into_iter().map(|k| k.text).collect()
    }

    /// Positions are exactly 0..n in list order.
    fn assert_contiguous(store: &Store) {
        let positions: Vec<u32> = store.keywords().expect("keywords").iter().map(|k| k.position).collect();
        let expected: Vec<u32> = (0..positions.len() as u32).collect();
        assert_eq!(positions, expected);
    }

    fn pragma<T: FromSql>(store: &Store, name: &str) -> T {
        store.conn.pragma_query_value(None, name, |row| row.get(0)).expect("pragma")
    }

    // ── Opening and migrations ──────────────────────────────────────────────────────────────

    #[test]
    fn store_can_move_between_threads() {
        // The engine keeps it behind a mutex and uses it from host threads and the runtime.
        fn assert_send<T: Send>() {}
        assert_send::<Store>();
    }

    #[test]
    fn opens_a_new_file_migrated_with_wal_foreign_keys_and_busy_timeout() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open(&dir.path().join("autopaper.sqlite3")).expect("open");
        assert_eq!(pragma::<i64>(&store, "user_version"), MIGRATIONS.len() as i64);
        assert_eq!(pragma::<String>(&store, "journal_mode"), "wal");
        assert_eq!(pragma::<i64>(&store, "foreign_keys"), 1);
        assert_eq!(pragma::<i64>(&store, "busy_timeout"), 5000);
    }

    #[test]
    fn reopening_is_idempotent_and_keeps_data() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("autopaper.sqlite3");
        {
            let mut store = Store::open(&path).expect("first open");
            store.upsert_keyword("fog", KeywordWeight::Must, T0).expect("keyword");
            store.insert_generation(&made("a", 0)).expect("insert");
            store.save_settings(&Settings { paused: true, ..Settings::default() }).expect("settings");
        }
        for _ in 0..2 {
            let store = Store::open(&path).expect("reopen");
            assert_eq!(pragma::<i64>(&store, "user_version"), MIGRATIONS.len() as i64);
            assert_eq!(keyword_texts(&store), ["fog"]);
            assert_eq!(store.generation("a").expect("read"), Some(made("a", 0)));
            assert!(store.settings().expect("settings").paused);
        }
    }

    #[test]
    fn migrates_an_existing_empty_database_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("autopaper.sqlite3");
        std::fs::File::create(&path).expect("empty file");
        let store = Store::open(&path).expect("open");
        assert_eq!(pragma::<i64>(&store, "user_version"), MIGRATIONS.len() as i64);
        assert!(store.keywords().expect("keywords").is_empty());
        assert_eq!(store.generation_count().expect("count"), 0);
    }

    #[test]
    fn a_failed_migration_rolls_back_with_its_version() {
        let mut conn = Connection::open_in_memory().expect("connection");
        let migrations = sql(&["CREATE TABLE first (x INTEGER);", "CREATE TABLE second (x INTEGER); SELECT nonsense(;"]);
        assert!(migrate(&mut conn, &migrations).is_err());
        assert_eq!(schema_version(&conn).expect("version"), 1);
        let second: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'second'", [], |row| row.get(0))
            .expect("schema");
        assert_eq!(second, 0);

        // Fixed, it resumes where it stopped.
        let fixed = sql(&["CREATE TABLE first (x INTEGER);", "CREATE TABLE second (x INTEGER);"]);
        migrate(&mut conn, &fixed).expect("resume");
        assert_eq!(schema_version(&conn).expect("version"), 2);
    }

    /// SQL-only migrations.
    fn sql(steps: &[&'static str]) -> Vec<Migration> {
        steps.iter().map(|sql| Migration { sql, data: None }).collect()
    }

    #[test]
    fn a_database_from_a_newer_build_opens_without_migrating() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("autopaper.sqlite3");
        let newer = MIGRATIONS.len() as i64 + 1;
        {
            let store = Store::open(&path).expect("open");
            store.conn.execute_batch("ALTER TABLE keywords ADD COLUMN colour TEXT;").expect("newer schema");
            store.conn.pragma_update(None, "user_version", newer).expect("stamp");
        }
        let mut store = Store::open(&path).expect("reopen");
        assert_eq!(pragma::<i64>(&store, "user_version"), newer);
        store.upsert_keyword("fog", KeywordWeight::Maybe, T0).expect("still writable");
        assert_eq!(keyword_texts(&store), ["fog"]);
    }

    #[test]
    fn a_version_1_database_gains_least_similar_and_keeps_its_rows() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("autopaper.sqlite3");
        {
            // A database as the first build left it: migration 1 only, one generation.
            let mut conn = Connection::open(&path).expect("open");
            migrate(&mut conn, &MIGRATIONS[..1]).expect("version 1");
            let v1 = Store { conn };
            let columns = "id, created_at, \"trigger\", status, title, summary, concept_json, prompt, text_provider,
                text_model, image_provider, image_model, width, height, embedding, embedding_model, surprise, keywords_json";
            v1.conn
                .execute(
                    &format!(
                        "INSERT INTO generations ({columns}) VALUES ('a', 1, 'scheduled', 'ok', 'A', 'S', '{{}}', 'p',
                         'demo', 'demo', 'demo', 'demo', 1, 1, x'', 'm', 0.3, '[]')"
                    ),
                    [],
                )
                .expect("v1 row");
        }
        let mut store = Store::open(&path).expect("migrate");
        assert_eq!(pragma::<i64>(&store, "user_version"), MIGRATIONS.len() as i64);
        let old = store.generation("a").expect("read").expect("kept");
        assert!(!old.least_similar, "rows from before it read as novel");
        store.insert_generation(&StoredGeneration { least_similar: true, ..made("b", 1) }).expect("insert");
        assert!(store.generation("b").expect("read").expect("present").least_similar);
        let invalid = store.conn.execute("UPDATE generations SET least_similar = 2 WHERE id = 'b'", []);
        assert!(invalid.is_err(), "the CHECK constraint holds");
    }

    #[test]
    fn the_queries_use_their_indexes() {
        let store = store();
        let plan = |sql: &str| -> String {
            let mut statement = store.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).expect("plan");
            let details = statement
                .query_map(params![10, 0], |row| row.get::<_, String>(3))
                .expect("plan rows")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("plan details");
            details.join("; ")
        };
        let all = plan(history_query!(""));
        assert!(all.contains("generations_by_status_time") && !all.contains("TEMP B-TREE"), "{all}");
        let liked = plan(history_query!(" AND rating = 1"));
        assert!(liked.contains("generations_by_rating") && !liked.contains("TEMP B-TREE"), "{liked}");
        let current = plan(
            "SELECT id FROM generations WHERE last_shown_at IS NOT NULL
             ORDER BY last_shown_at DESC, rowid DESC LIMIT ?1 OFFSET ?2",
        );
        assert!(current.contains("generations_by_last_shown"), "{current}");
        let echoes = plan("SELECT id FROM generations WHERE echo_of = ?1 LIMIT ?2");
        assert!(echoes.contains("generations_by_echo_of"), "{echoes}");
        let rated = plan(
            "SELECT concept_json FROM generations WHERE rating = ?1 AND status = 'ok'
             ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        );
        assert!(rated.contains("generations_by_rating") && !rated.contains("TEMP B-TREE"), "{rated}");
        let narrow = plan(
            "SELECT least_similar FROM generations WHERE status = 'ok' AND echo_of IS NULL
             ORDER BY created_at DESC, rowid DESC LIMIT ?1 OFFSET ?2",
        );
        assert!(narrow.contains("generations_by_status_time") && !narrow.contains("TEMP B-TREE"), "{narrow}");
        let failures = plan("DELETE FROM generations WHERE status IN ('failed', 'refused') AND created_at < ?1 + ?2");
        assert!(failures.contains("generations_by_status_time"), "{failures}");
    }

    // ── Column encodings ────────────────────────────────────────────────────────────────────

    #[test]
    fn enum_columns_use_serde_snake_case_names() {
        fn check<T: ToSql + FromSql + Serialize + PartialEq + std::fmt::Debug>(store: &Store, values: &[T]) {
            for value in values {
                let stored: String = store.conn.query_row("SELECT ?1", [value], |row| row.get(0)).expect("encode");
                let serde = serde_json::to_value(value).expect("serde");
                assert_eq!(Some(stored.as_str()), serde.as_str());
                let back: T = store.conn.query_row("SELECT ?1", [value], |row| row.get(0)).expect("decode");
                assert_eq!(&back, value);
            }
        }
        let store = store();
        check(&store, &[KeywordWeight::Must, KeywordWeight::Maybe, KeywordWeight::Avoid]);
        check(&store, &[Trigger::Scheduled, Trigger::Manual, Trigger::DislikeReplace, Trigger::EchoRequest]);
        check(&store, &[GenerationStatus::Ok, GenerationStatus::Failed, GenerationStatus::Refused]);
        check(
            &store,
            &[
                ProviderKind::OpenAi,
                ProviderKind::Google,
                ProviderKind::Ollama,
                ProviderKind::OpenAiCompatible,
                ProviderKind::ComfyUi,
                ProviderKind::Demo,
            ],
        );
    }

    #[test]
    fn a_generation_round_trips_with_stable_column_encodings() {
        let mut store = store();
        let mut stored = with(made("e", 3), |g| {
            g.trigger = Trigger::EchoRequest;
            g.rating = Rating::Disliked;
            g.echo_of = Some("a".into());
            g.echo_note = Some("Echo of “Black ocean, silver structures” (March 2024): after a storm.".into());
            g.text_provider = ProviderKind::OpenAiCompatible;
            g.image_provider = ProviderKind::ComfyUi;
            g.cost_microusd = i64::MAX as u64;
            g.last_shown_at = Some(T0 + 4 * DAY);
            g.shown_count = 7;
            g.surprise = 0.8;
            g.keywords.push(KeywordSnapshot { text: "people".into(), weight: KeywordWeight::Avoid });
        });
        stored.phash = Some(u64::MAX - 1);
        stored.embedding = vec![1.0, -0.5, f32::MIN_POSITIVE, 0.0];
        store.insert_generation(&stored).expect("insert");
        assert_eq!(store.generation("e").expect("read"), Some(stored));

        let column = |name: &str| -> String {
            raw(&store, &format!("SELECT CAST({name} AS TEXT) FROM generations WHERE id = ?1"), "e")
        };
        assert_eq!(column("\"trigger\""), "echo_request");
        assert_eq!(column("status"), "ok");
        assert_eq!(column("text_provider"), "open_ai_compatible");
        assert_eq!(column("image_provider"), "comfy_ui");
        assert_eq!(column("rating"), "-1");
        assert_eq!(column("rated_at"), (T0 + 3 * DAY).to_string());
        assert!(column("keywords_json").contains(r#""weight":"avoid""#));
        assert_eq!(column("title"), "e");
        assert_eq!(column("prompt"), "A desktop wallpaper: e.");
        let phash: i64 = raw(&store, "SELECT phash FROM generations WHERE id = ?1", "e");
        assert_eq!(phash, -2);
        let blob: Vec<u8> = raw(&store, "SELECT embedding FROM generations WHERE id = ?1", "e");
        assert_eq!(blob.len(), 16);
        assert_eq!(blob[..8], [0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0xbf]);
    }

    #[test]
    fn a_failed_generation_round_trips_without_embedding_or_hash() {
        let mut store = store();
        store.insert_generation(&failed("f", 0)).expect("insert");
        assert_eq!(store.generation("f").expect("read"), Some(failed("f", 0)));
        assert_eq!(store.generation("missing").expect("read"), None);
    }

    #[test]
    fn insert_rejects_a_duplicate_id_and_an_unstorable_cost() {
        let mut store = store();
        store.insert_generation(&made("a", 0)).expect("insert");
        assert!(matches!(store.insert_generation(&made("a", 1)), Err(AutoPaperError::InvalidInput { .. })));
        let costly = with(made("b", 0), |g| g.cost_microusd = u64::MAX);
        assert!(matches!(store.insert_generation(&costly), Err(AutoPaperError::InvalidInput { .. })));
        assert_eq!(store.generation_count().expect("count"), 1);
    }

    #[test]
    fn concepts_from_older_or_newer_builds_still_read() {
        let mut store = store();
        store.insert_generation(&made("a", 0)).expect("insert");
        store
            .conn
            .execute(
                r#"UPDATE generations
                   SET concept_json = '{"title":"Old","summary":"From an older build.","lens":"35mm"}'
                   WHERE id = 'a'"#,
                [],
            )
            .expect("rewrite concept");
        let read = store.generation("a").expect("read").expect("present").generation.concept;
        assert_eq!(read.title, "Old");
        assert_eq!(read.summary, "From an older build.");
        assert!(read.elements.is_empty() && read.prompt.is_empty());
    }

    #[test]
    fn unreadable_rows_are_storage_errors_not_panics() {
        let mut store = store();
        insert_all(&mut store, vec![made("a", 0), made("b", 1)]);
        let execute = |sql: &str| store.conn.execute(sql, []).expect("corrupt");
        execute("UPDATE generations SET text_provider = 'anthropic' WHERE id = 'a'");
        execute("UPDATE generations SET embedding = x'0000803f00' WHERE id = 'b'");
        assert!(matches!(store.generation("a"), Err(AutoPaperError::Storage { .. })));
        assert!(matches!(store.history(HistoryFilter::All, 10, 0), Err(AutoPaperError::Storage { .. })));
        assert!(matches!(store.memory(), Err(AutoPaperError::Storage { .. })));
        execute("UPDATE generations SET concept_json = '[1, 2]' WHERE id = 'b'");
        assert!(matches!(store.generation("b"), Err(AutoPaperError::Storage { .. })));
    }

    // ── Keywords ────────────────────────────────────────────────────────────────────────────

    #[test]
    fn keywords_append_in_order_with_positions_from_zero() {
        let mut store = store();
        let fog = store.upsert_keyword("fog", KeywordWeight::Must, T0).expect("fog");
        let rain = store.upsert_keyword("  rain   at  night ", KeywordWeight::Maybe, T0 + 1).expect("rain");
        assert_eq!((fog.position, rain.position), (0, 1));
        assert_eq!(rain.text, "rain at night");
        assert_eq!(rain.created_at, T0 + 1);
        assert_eq!(store.keywords().expect("keywords"), vec![fog, rain]);
    }

    #[test]
    fn upserting_existing_text_updates_its_weight_case_insensitively() {
        let mut store = store();
        let fog = store.upsert_keyword("Fog", KeywordWeight::Must, T0).expect("fog");
        store.upsert_keyword("sea", KeywordWeight::Maybe, T0).expect("sea");
        let again = store.upsert_keyword("FOG", KeywordWeight::Avoid, T0 + 9).expect("again");
        assert_eq!(again, Keyword { weight: KeywordWeight::Avoid, ..fog });
        assert_eq!(keyword_texts(&store), ["Fog", "sea"]);

        // Beyond ASCII, where SQLite's NOCASE stops.
        let foam = store.upsert_keyword("Écume", KeywordWeight::Maybe, T0).expect("foam");
        let foam_again = store.upsert_keyword("écume", KeywordWeight::Must, T0).expect("foam again");
        assert_eq!(foam_again.id, foam.id);
        assert_eq!(keyword_texts(&store), ["Fog", "sea", "Écume"]);
    }

    #[test]
    fn upsert_rejects_invalid_text() {
        let mut store = store();
        for text in ["", "   ", "!!!", &"x".repeat(Keyword::MAX_LEN + 1)] {
            assert!(matches!(
                store.upsert_keyword(text, KeywordWeight::Must, T0),
                Err(AutoPaperError::InvalidInput { .. })
            ));
        }
        assert!(store.keywords().expect("keywords").is_empty());
    }

    #[test]
    fn keywords_stop_at_max_count_but_existing_ones_still_update() {
        let mut store = store();
        for n in 0..Keyword::MAX_COUNT {
            store.upsert_keyword(&format!("word {n}"), KeywordWeight::Maybe, T0).expect("within limit");
        }
        assert!(matches!(
            store.upsert_keyword("one too many", KeywordWeight::Maybe, T0),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::TooManyKeywords, .. })
        ));
        let updated = store.upsert_keyword("WORD 3", KeywordWeight::Avoid, T0).expect("existing");
        assert_eq!((updated.text.as_str(), updated.weight), ("word 3", KeywordWeight::Avoid));
        assert_eq!(store.keywords().expect("keywords").len(), Keyword::MAX_COUNT);
        assert_contiguous(&store);
    }

    #[test]
    fn set_keyword_weight_changes_only_the_weight() {
        let mut store = store();
        let fog = store.upsert_keyword("fog", KeywordWeight::Must, T0).expect("fog");
        store.set_keyword_weight(&fog.id, KeywordWeight::Avoid).expect("set");
        assert_eq!(store.keywords().expect("keywords"), vec![Keyword { weight: KeywordWeight::Avoid, ..fog }]);
        assert!(matches!(store.set_keyword_weight("nope", KeywordWeight::Must), Err(AutoPaperError::NotFound)));
    }

    #[test]
    fn rename_keyword_checks_ids_and_duplicates() {
        let mut store = store();
        let fog = store.upsert_keyword("fog", KeywordWeight::Must, T0).expect("fog");
        store.upsert_keyword("sea", KeywordWeight::Maybe, T0).expect("sea");

        let renamed = store.rename_keyword(&fog.id, " sea  fog ").expect("rename");
        assert_eq!(renamed, Keyword { text: "sea fog".into(), ..fog.clone() });
        assert!(matches!(
            store.rename_keyword(&fog.id, "SEA"),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::DuplicateKeyword, .. })
        ));
        assert!(matches!(
            store.rename_keyword(&fog.id, "  "),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::KeywordEmpty, .. })
        ));
        assert!(matches!(
            store.rename_keyword(&fog.id, &"x".repeat(Keyword::MAX_LEN + 1)),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::KeywordTooLong, .. })
        ));
        assert!(matches!(store.rename_keyword("nope", "storm"), Err(AutoPaperError::NotFound)));
        let recased = store.rename_keyword(&fog.id, "Sea Fog").expect("own text, new case");
        assert_eq!(recased.text, "Sea Fog");
        assert_eq!(keyword_texts(&store), ["Sea Fog", "sea"]);
    }

    #[test]
    fn move_keyword_reorders_and_clamps() {
        let mut store = store();
        let ids: Vec<String> = ["a", "b", "c", "d"]
            .iter()
            .map(|text| store.upsert_keyword(text, KeywordWeight::Maybe, T0).expect("keyword").id)
            .collect();
        store.move_keyword(&ids[0], 2).expect("forward");
        assert_eq!(keyword_texts(&store), ["b", "c", "a", "d"]);
        store.move_keyword(&ids[3], 0).expect("backward");
        assert_eq!(keyword_texts(&store), ["d", "b", "c", "a"]);
        store.move_keyword(&ids[3], u32::MAX).expect("clamped");
        assert_eq!(keyword_texts(&store), ["b", "c", "a", "d"]);
        store.move_keyword(&ids[2], 1).expect("same place");
        assert_eq!(keyword_texts(&store), ["b", "c", "a", "d"]);
        assert_contiguous(&store);
        assert!(matches!(store.move_keyword("nope", 0), Err(AutoPaperError::NotFound)));
    }

    #[test]
    fn delete_keyword_renumbers_without_gaps() {
        let mut store = store();
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|text| store.upsert_keyword(text, KeywordWeight::Must, T0).expect("keyword").id)
            .collect();
        store.delete_keyword(&ids[1]).expect("delete");
        assert_eq!(keyword_texts(&store), ["a", "c"]);
        assert_contiguous(&store);
        assert!(matches!(store.delete_keyword(&ids[1]), Err(AutoPaperError::NotFound)));
        let d = store.upsert_keyword("d", KeywordWeight::Must, T0).expect("append after delete");
        assert_eq!(d.position, 2);
        assert_contiguous(&store);
    }

    // ── Settings and state ──────────────────────────────────────────────────────────────────

    fn custom_settings() -> Settings {
        Settings {
            surprise: 0.8,
            cadence: Cadence::Every3Hours,
            paused: true,
            quiet_period: QuietPeriod::TwoYears,
            echoes: EchoFrequency::Often,
            text_provider: ProviderSelection {
                kind: ProviderKind::Ollama,
                model: "qwen3".into(),
                base_url: Some("http://127.0.0.1:11434".into()),
            },
            image_provider: ProviderSelection { kind: ProviderKind::OpenAi, model: String::new(), base_url: None },
            image_quality: ImageQuality::Standard,
            monthly_budget_cents: None,
            fallback: Fallback::KeepCurrent,
            replace_disliked: false,
            set_lock_screen: false,
            storage_limit_mb: 512,
            comfyui_workflow: Some(r#"{"3": {"inputs": {"text": "{{prompt}}"}}}"#.into()),
        }
    }

    fn settings_rows(store: &Store) -> Vec<(String, String)> {
        let mut statement = store.conn.prepare("SELECT key, value FROM settings ORDER BY key").expect("rows");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("collect")
    }

    fn set_setting_row(store: &Store, key: &str, json: &str) {
        store
            .conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, json],
            )
            .expect("write row");
    }

    #[test]
    fn settings_default_until_saved_then_round_trip() {
        let mut store = store();
        assert_eq!(store.settings().expect("defaults"), Settings::default());
        store.save_settings(&custom_settings()).expect("save");
        assert_eq!(store.settings().expect("read"), custom_settings());
        store.save_settings(&Settings::default()).expect("save defaults");
        assert_eq!(store.settings().expect("read"), Settings::default());
    }

    #[test]
    fn settings_are_one_json_row_per_field() {
        let mut store = store();
        store.save_settings(&custom_settings()).expect("save");
        let rows = settings_rows(&store);
        let Value::Object(fields) = serde_json::to_value(Settings::default()).expect("json") else {
            panic!("settings serialise to an object");
        };
        let keys: Vec<&str> = rows.iter().map(|(key, _)| key.as_str()).collect();
        let mut expected: Vec<&str> = fields.keys().map(String::as_str).collect();
        expected.sort_unstable();
        assert_eq!(keys, expected);
        let value_of = |key: &str| rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(value_of("cadence").as_deref(), Some(r#""every3_hours""#));
        assert_eq!(value_of("monthly_budget_cents").as_deref(), Some("null"));
        assert_eq!(value_of("paused").as_deref(), Some("true"));
    }

    #[test]
    fn missing_or_unreadable_settings_fall_back_one_field_at_a_time() {
        let mut store = store();
        store.save_settings(&custom_settings()).expect("save");
        store.conn.execute("DELETE FROM settings WHERE key = 'quiet_period'", []).expect("drop row");
        set_setting_row(&store, "cadence", r#""every_two_hours""#); // a choice from a newer build
        set_setting_row(&store, "storage_limit_mb", "-5");
        // Surprise is the active mood's: its settings row is only kept for older builds.
        set_setting_row(&store, "surprise", "{not json");

        let expected = Settings {
            quiet_period: Settings::default().quiet_period,
            cadence: Settings::default().cadence,
            storage_limit_mb: Settings::default().storage_limit_mb,
            ..custom_settings()
        };
        assert_eq!(store.settings().expect("read"), expected);
    }

    /// The settings a later build might have: everything today, plus a new field with its own default.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct NextSettings {
        #[serde(flatten)]
        today: Settings,
        dim_at_night: bool,
    }

    impl Default for NextSettings {
        fn default() -> Self {
            Self { today: Settings::default(), dim_at_night: true }
        }
    }

    #[test]
    fn a_newly_added_settings_field_gets_its_default_from_an_older_database() {
        let mut store = store();
        store.save_settings(&custom_settings()).expect("older build saves");
        let next: NextSettings = decode_fields(settings_rows(&store)).expect("newer build reads");
        assert_eq!(next, NextSettings { today: custom_settings(), dim_at_night: true });
    }

    #[test]
    fn an_older_build_ignores_and_keeps_a_newer_builds_settings() {
        let mut store = store();
        let newer = NextSettings { today: custom_settings(), dim_at_night: false };
        for (key, value) in encode_fields(&newer).expect("encode") {
            set_setting_row(&store, &key, &value);
        }
        let mood = store.active_mood_id().expect("mood");
        store.set_mood_surprise(&mood, custom_settings().surprise).expect("its surprise");
        assert_eq!(store.settings().expect("older build reads"), custom_settings());

        let mut changed = custom_settings();
        changed.paused = false;
        store.save_settings(&changed).expect("older build saves");
        let next: NextSettings = decode_fields(settings_rows(&store)).expect("newer build reads again");
        assert_eq!(next, NextSettings { today: changed, dim_at_night: false });
    }

    #[test]
    fn state_values_round_trip_as_json() {
        let mut store = store();
        assert_eq!(store.state_get("backoff_until").expect("get"), None);
        store.state_set("backoff_until", &serde_json::json!(T0 + 600)).expect("set");
        store.state_set("consecutive_failures", &serde_json::json!(2)).expect("set");
        store.state_set("consecutive_failures", &serde_json::json!(3)).expect("overwrite");
        store.state_set("note", &serde_json::json!({"text": "雨の夜 🌧", "ok": true})).expect("set object");
        assert_eq!(store.state_get("backoff_until").expect("get"), Some(serde_json::json!(T0 + 600)));
        assert_eq!(store.state_get("consecutive_failures").expect("get"), Some(serde_json::json!(3)));
        assert_eq!(store.state_get("note").expect("get"), Some(serde_json::json!({"text": "雨の夜 🌧", "ok": true})));

        store.conn.execute("UPDATE state SET value = '{oops' WHERE key = 'note'", []).expect("corrupt");
        assert!(matches!(store.state_get("note"), Err(AutoPaperError::Storage { .. })));
    }

    // ── History, showing, rating ────────────────────────────────────────────────────────────

    #[test]
    fn history_filters_and_pages_newest_first_without_failures() {
        let mut store = store();
        insert_all(
            &mut store,
            vec![
                made("a", 0),
                with(made("b", 1), |g| g.rating = Rating::Liked),
                failed("x", 2),
                with(made("c", 3), |g| g.rating = Rating::Disliked),
                with(made("d", 4), |g| g.echo_of = Some("a".into())),
                with(failed("y", 5), |g| g.status = GenerationStatus::Refused),
                with(made("e", 6), |g| {
                    g.rating = Rating::Liked;
                    g.echo_of = Some("b".into());
                }),
                with(made("f", 6), |g| g.image_path = None), // same second as e, inserted later; pruned
            ],
        );
        let page = |filter, limit, offset| store.history(filter, limit, offset).expect("history");
        assert_eq!(ids(&page(HistoryFilter::All, 50, 0)), ["f", "e", "d", "c", "b", "a"]);
        assert_eq!(ids(&page(HistoryFilter::Liked, 50, 0)), ["e", "b"]);
        assert_eq!(ids(&page(HistoryFilter::Disliked, 50, 0)), ["c"]);
        assert_eq!(ids(&page(HistoryFilter::Echoes, 50, 0)), ["e", "d"]);
        assert_eq!(ids(&page(HistoryFilter::All, 2, 0)), ["f", "e"]);
        assert_eq!(ids(&page(HistoryFilter::All, 2, 2)), ["d", "c"]);
        assert_eq!(ids(&page(HistoryFilter::All, 2, 4)), ["b", "a"]);
        assert!(page(HistoryFilter::All, 2, 6).is_empty());
        assert!(page(HistoryFilter::All, 0, 0).is_empty());
        assert_eq!(store.generation_count().expect("count"), 6);
    }

    #[test]
    fn current_follows_mark_shown() {
        let mut store = store();
        insert_all(&mut store, vec![made("a", 0), made("b", 1)]);
        assert_eq!(store.current().expect("current"), None);
        store.mark_shown("a", T0 + 10).expect("show a");
        store.mark_shown("b", T0 + 20).expect("show b");
        assert_eq!(store.current().expect("current").map(|g| g.id), Some("b".into()));
        store.mark_shown("a", T0 + 30).expect("show a again");
        let current = store.current().expect("current").expect("something shown");
        assert_eq!((current.id.as_str(), current.last_shown_at, current.shown_count), ("a", Some(T0 + 30), 2));
        assert!(matches!(store.mark_shown("nope", T0), Err(AutoPaperError::NotFound)));
    }

    #[test]
    fn set_rating_returns_the_previous_rating() {
        let mut store = store();
        insert_all(&mut store, vec![made("a", 0), made("b", 1)]);
        let rated_at =
            |store: &Store| -> Option<i64> { raw(store, "SELECT rated_at FROM generations WHERE id = ?1", "a") };

        assert_eq!(store.set_rating("a", Rating::Liked, T0 + 1).expect("like"), Rating::Unrated);
        assert_eq!(rated_at(&store), Some(T0 + 1));
        assert_eq!(store.set_rating("a", Rating::Liked, T0 + 2).expect("like again"), Rating::Liked);
        assert_eq!(rated_at(&store), Some(T0 + 1));
        assert_eq!(store.set_rating("a", Rating::Disliked, T0 + 3).expect("dislike"), Rating::Liked);
        assert_eq!(store.rated_count().expect("count"), 1);
        assert_eq!(store.set_rating("b", Rating::Liked, T0 + 4).expect("like b"), Rating::Unrated);
        assert_eq!(store.rated_count().expect("count"), 2);
        assert_eq!(store.set_rating("a", Rating::Unrated, T0 + 5).expect("clear"), Rating::Disliked);
        assert_eq!(rated_at(&store), None);
        assert_eq!(store.rated_count().expect("count"), 1);
        assert_eq!(store.generation("a").expect("read").expect("present").generation.rating, Rating::Unrated);
        assert!(matches!(store.set_rating("nope", Rating::Liked, T0), Err(AutoPaperError::NotFound)));
    }

    // ── Memory and lineage ──────────────────────────────────────────────────────────────────

    #[test]
    fn memory_is_oldest_first_and_only_successful() {
        let mut store = store();
        insert_all(
            &mut store,
            vec![
                with(made("b", 2), |g| g.rating = Rating::Liked),
                failed("x", 1),
                made("a", 0),
                with(made("c", 3), |g| g.echo_of = Some("a".into())),
            ],
        );
        let memory = store.memory().expect("memory");
        assert_eq!(memory.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(
            memory[1],
            MemoryRow {
                id: "b".into(),
                created_at: T0 + 2 * DAY,
                title: "b".into(),
                summary: "b, as a quiet landscape.".into(),
                embedding: vec![0.6, 0.8],
                embedding_model: "test-embedder".into(),
                rating: Rating::Liked,
                echo_of: None,
                mood_id: None,
            }
        );
        assert_eq!(memory[2].echo_of.as_deref(), Some("a"));
    }

    #[test]
    fn update_embedding_replaces_vector_and_model() {
        let mut store = store();
        store.insert_generation(&made("a", 0)).expect("insert");
        store.update_embedding("a", &[0.0, 0.0, 1.0], "bge-small-en-v1.5").expect("update");
        let row = &store.memory().expect("memory")[0];
        assert_eq!(
            (row.embedding.as_slice(), row.embedding_model.as_str()),
            (&[0.0, 0.0, 1.0][..], "bge-small-en-v1.5")
        );
        assert!(matches!(store.update_embedding("nope", &[1.0], "m"), Err(AutoPaperError::NotFound)));
    }

    #[test]
    fn recent_summaries_are_newest_first() {
        let mut store = store();
        assert!(store.recent_summaries(5).expect("empty").is_empty());
        insert_all(&mut store, vec![made("a", 0), made("b", 1), failed("x", 2), made("c", 3)]);
        assert_eq!(store.recent_summaries(2).expect("two"), ["c, as a quiet landscape.", "b, as a quiet landscape."]);
        assert_eq!(store.recent_summaries(10).expect("all").len(), 3);
        assert!(store.recent_summaries(0).expect("none").is_empty());
    }

    #[test]
    fn lineage_spans_a_three_level_echo_chain() {
        let mut store = store();
        let echo = |id: &str, day: i64, of: &str| with(made(id, day), |g| g.echo_of = Some(of.to_string()));
        insert_all(
            &mut store,
            vec![
                made("root", 0),
                made("other", 1),
                echo("child", 200, "root"),
                echo("sibling", 300, "root"),
                echo("grandchild", 400, "child"),
                with(failed("failed-echo", 401), |g| g.echo_of = Some("child".into())),
                echo("unrelated-echo", 402, "other"),
            ],
        );
        let expected = ["root", "child", "sibling", "grandchild"];
        for id in expected {
            assert_eq!(ids(&store.lineage(id).expect("lineage")), expected, "from {id}");
        }
        assert_eq!(ids(&store.lineage("other").expect("lineage")), ["other", "unrelated-echo"]);
        assert!(matches!(store.lineage("nope"), Err(AutoPaperError::NotFound)));

        // Deleting the original keeps its echoes together.
        store.delete_generation("root").expect("delete");
        assert_eq!(ids(&store.lineage("grandchild").expect("lineage")), ["child", "sibling", "grandchild"]);
        assert_eq!(
            store.generation("child").expect("read").expect("present").generation.echo_of.as_deref(),
            Some("root")
        );
    }

    #[test]
    fn has_relatives_matches_the_lineage() {
        let mut store = store();
        let echo = |id: &str, day: i64, of: &str| with(made(id, day), |g| g.echo_of = Some(of.to_string()));
        insert_all(
            &mut store,
            vec![
                made("alone", 0),
                made("root", 1),
                echo("child", 200, "root"),
                echo("grandchild", 300, "child"),
                made("lonely-root", 2),
                with(failed("failed-echo", 301), |g| g.echo_of = Some("lonely-root".into())),
                made("hidden-root", 3),
                echo("echo-of-hidden", 302, "hidden-root"),
            ],
        );
        for id in ["alone", "root", "child", "grandchild", "lonely-root", "failed-echo", "hidden-root", "echo-of-hidden"] {
            let listed = store.lineage(id).expect("lineage").iter().any(|g| g.id != id);
            assert_eq!(store.has_relatives(id).expect("relatives"), listed, "{id}");
        }
        assert!(!store.has_relatives("alone").unwrap());
        assert!(store.has_relatives("root").unwrap() && store.has_relatives("grandchild").unwrap());
        assert!(!store.has_relatives("lonely-root").unwrap(), "a failed echo isn't in history");
        assert!(matches!(store.has_relatives("nope"), Err(AutoPaperError::NotFound)));

        // An original cleared from history leaves its echo alone; deleting one keeps the rest together.
        store.conn.execute("UPDATE generations SET hidden = 1 WHERE id = 'hidden-root'", []).unwrap();
        assert!(!store.has_relatives("echo-of-hidden").unwrap());
        store.delete_generation("root").expect("delete");
        assert!(store.has_relatives("grandchild").unwrap());
        store.delete_generation("child").expect("delete");
        assert!(!store.has_relatives("grandchild").unwrap());
    }

    #[test]
    fn lineage_survives_a_corrupt_cycle() {
        let mut store = store();
        insert_all(
            &mut store,
            vec![
                with(made("a", 0), |g| g.echo_of = Some("b".into())),
                with(made("b", 1), |g| g.echo_of = Some("a".into())),
            ],
        );
        assert_eq!(ids(&store.lineage("a").expect("lineage")), ["a", "b"]);
    }

    // ── Scheduling, revisits, pruning ───────────────────────────────────────────────────────

    #[test]
    fn last_new_at_counts_every_successful_generation_whatever_its_trigger() {
        let mut store = store();
        assert_eq!(store.last_new_at().expect("empty"), None);
        insert_all(&mut store, vec![made("a", 0), with(made("echo", 2), |g| g.echo_of = Some("a".into()))]);
        assert_eq!(store.last_new_at().expect("scheduled"), Some(T0 + 2 * DAY));
        for (id, day, trigger) in
            [("manual", 3, Trigger::Manual), ("replace", 4, Trigger::DislikeReplace), ("asked", 6, Trigger::EchoRequest)]
        {
            store.insert_generation(&with(made(id, day), |g| g.trigger = trigger)).expect("insert");
            assert_eq!(store.last_new_at().expect("last"), Some(T0 + day * DAY), "{id}");
        }
        insert_all(&mut store, vec![failed("failed", 7), with(failed("refused", 8), |g| g.status = GenerationStatus::Refused)]);
        assert_eq!(store.last_new_at().expect("failures don't count"), Some(T0 + 6 * DAY));
        // A revisit only marks an old one shown; cleared history still counts (it was made then).
        store.mark_shown("a", T0 + 9 * DAY).expect("revisit");
        store.clear_history(true).expect("clear");
        assert_eq!(store.last_new_at().expect("after a revisit and a clear"), Some(T0 + 6 * DAY));
    }

    #[test]
    fn recent_least_similar_reads_the_newest_new_wallpapers() {
        let mut store = store();
        assert!(store.recent_least_similar(5, None).expect("empty").is_empty());
        let fell_back = |stored: StoredGeneration| StoredGeneration { least_similar: true, ..stored };
        insert_all(
            &mut store,
            vec![
                fell_back(made("old", 0)),
                made("a", 1),
                fell_back(with(made("b", 2), |g| g.trigger = Trigger::Manual)),
                fell_back(with(made("echo", 3), |g| g.echo_of = Some("a".into()))),
                fell_back(failed("failed", 4)),
                fell_back(with(made("c", 5), |g| g.trigger = Trigger::DislikeReplace)),
                made("d", 6),
            ],
        );
        assert_eq!(store.recent_least_similar(3, None).expect("three"), [false, true, true], "d, c, b: no echo, no failure");
        assert_eq!(store.recent_least_similar(10, None).expect("all"), [false, true, true, false, true]);
        store.clear_history(true).expect("clear");
        assert_eq!(store.recent_least_similar(3, None).expect("memory kept"), [false, true, true]);
        assert!(store.generation("c").expect("read").expect("present").least_similar);
    }

    #[test]
    fn delete_unsuccessful_before_removes_only_old_failures() {
        let mut store = store();
        insert_all(
            &mut store,
            vec![
                made("old-ok", 0),
                failed("old-failed", 1),
                with(failed("old-refused", 2), |g| g.status = GenerationStatus::Refused),
                failed("recent-failed", 40),
                made("recent-ok", 41),
            ],
        );
        assert_eq!(store.delete_unsuccessful_before(T0 + 40 * DAY).expect("delete"), 2);
        assert_eq!(store.generation("old-failed").expect("read"), None);
        assert_eq!(store.generation("old-refused").expect("read"), None);
        for kept in ["old-ok", "recent-failed", "recent-ok"] {
            assert!(store.generation(kept).expect("read").is_some(), "{kept}");
        }
        assert_eq!(store.delete_unsuccessful_before(T0 + 40 * DAY).expect("again"), 0);
    }

    #[test]
    fn liked_with_images_puts_least_recently_shown_first() {
        let mut store = store();
        let liked = |id: &str, day: i64, shown: Option<i64>| {
            with(made(id, day), |g| {
                g.rating = Rating::Liked;
                g.last_shown_at = shown;
            })
        };
        insert_all(
            &mut store,
            vec![
                liked("shown-late", 0, Some(T0 + 90 * DAY)),
                liked("shown-early", 1, Some(T0 + 10 * DAY)),
                liked("never-shown-new", 5, None),
                liked("never-shown-old", 2, None),
                with(liked("pruned", 3, None), |g| g.image_path = None),
                made("unrated", 4),
                with(failed("failed", 6), |g| g.rating = Rating::Liked),
            ],
        );
        assert_eq!(
            ids(&store.liked_with_images().expect("liked")),
            ["never-shown-old", "never-shown-new", "shown-early", "shown-late"]
        );
    }

    #[test]
    fn set_image_paths_sets_and_clears() {
        let mut store = store();
        store.insert_generation(&made("a", 0)).expect("insert");
        store.set_image_paths("a", None, None).expect("clear");
        let generation = store.generation("a").expect("read").expect("present").generation;
        assert_eq!((generation.image_path, generation.thumb_path), (None, None));
        store.set_image_paths("a", Some("images/2027/a.webp"), Some("thumbs/a.jpg")).expect("set");
        let generation = store.generation("a").expect("read").expect("present").generation;
        assert_eq!(generation.image_path.as_deref(), Some("images/2027/a.webp"));
        assert_eq!(generation.thumb_path.as_deref(), Some("thumbs/a.jpg"));
        assert!(matches!(store.set_image_paths("nope", None, None), Err(AutoPaperError::NotFound)));
    }

    #[test]
    fn prunable_is_oldest_first_and_never_liked() {
        let mut store = store();
        insert_all(
            &mut store,
            vec![
                with(made("disliked", 2), |g| g.rating = Rating::Disliked),
                made("oldest", 0),
                with(made("liked", 1), |g| g.rating = Rating::Liked),
                with(made("pruned", 3), |g| g.image_path = None),
                made("newest", 4),
            ],
        );
        assert_eq!(ids(&store.prunable().expect("prunable")), ["oldest", "disliked", "newest"]);
    }

    #[test]
    fn rated_concepts_include_cleared_history_newest_first() {
        let mut store = store();
        insert_all(
            &mut store,
            vec![
                with(made("old", 0), |g| g.rating = Rating::Liked),
                with(made("new", 2), |g| g.rating = Rating::Liked),
                with(made("disliked", 1), |g| g.rating = Rating::Disliked),
                made("unrated", 3),
                with(failed("failed", 4), |g| g.rating = Rating::Liked),
            ],
        );
        store.clear_history(true).expect("clear");
        let titles = |rating, limit| -> Vec<String> {
            store.rated_concepts(rating, limit).expect("rated").into_iter().map(|concept| concept.title).collect()
        };
        assert_eq!(titles(Rating::Liked, 10), [concept("new").title, concept("old").title]);
        assert_eq!(titles(Rating::Liked, 1), [concept("new").title]);
        assert_eq!(titles(Rating::Disliked, 10), [concept("disliked").title]);
        assert_eq!(store.generation_ids().expect("ids").len(), 5);
    }

    #[test]
    fn delete_generation_removes_one_row() {
        let mut store = store();
        insert_all(&mut store, vec![made("a", 0), made("b", 1)]);
        store.delete_generation("a").expect("delete");
        assert_eq!(store.generation("a").expect("read"), None);
        assert_eq!(store.generation_count().expect("count"), 1);
        assert!(matches!(store.delete_generation("a"), Err(AutoPaperError::NotFound)));
    }

    // ── Clearing ────────────────────────────────────────────────────────────────────────────

    fn filled_store() -> Store {
        let mut store = store();
        store.upsert_keyword("fog", KeywordWeight::Must, T0).expect("keyword");
        store.save_settings(&custom_settings()).expect("settings");
        store.state_set("consecutive_failures", &serde_json::json!(1)).expect("state");
        insert_all(&mut store, vec![with(made("a", 0), |g| g.rating = Rating::Liked), made("b", 1)]);
        store.mark_shown("b", T0 + DAY).expect("shown");
        store
            .save_taste_rows(&[TasteRow { feature: "fog".into(), likes: 1.0, dislikes: 0.0, updated_at: T0 }])
            .expect("taste");
        store.add_spend("2026-10", 100_000, 1).expect("spend");
        store
    }

    #[test]
    fn clear_history_keeping_memory_hides_history_and_drops_image_paths() {
        let mut store = filled_store();
        let memory_before = store.memory().expect("memory");
        store.clear_history(true).expect("clear");
        assert_eq!(store.memory().expect("memory"), memory_before);
        assert!(store.history(HistoryFilter::All, 10, 0).expect("history").is_empty());
        assert_eq!(store.current().expect("current"), None);
        assert!(store.lineage("a").expect("lineage").is_empty());
        assert!(store.liked_with_images().expect("liked").is_empty());
        assert_eq!(store.hidden_ids().expect("hidden"), HashSet::from(["a".to_string(), "b".to_string()]));
        for id in ["a", "b"] {
            let generation = store.generation(id).expect("read").expect("kept").generation;
            assert_eq!((generation.image_path, generation.thumb_path), (None, None));
        }
        assert_eq!(store.generation("a").expect("read").expect("kept").phash, Some(0x0123_4567_89ab_cdef));
        assert_eq!(store.recent_summaries(10).expect("summaries").len(), 2, "memory still avoids repeats");
        assert_eq!(store.generation_count().expect("count"), 2);

        assert_eq!(store.rated_count().expect("ratings kept"), 1);
        assert_eq!(store.taste_rows().expect("taste kept").len(), 1);
        assert_eq!(store.spend("2026-10").expect("spend kept"), (100_000, 1));
        assert!(store.prunable().expect("prunable").is_empty());
        // New generations show up as usual.
        store.insert_generation(&made("c", 2)).expect("insert");
        assert_eq!(ids(&store.history(HistoryFilter::All, 10, 0).expect("history")), ["c"]);
    }

    #[test]
    fn clear_history_without_memory_forgets_generations_and_taste_but_not_spend() {
        let mut store = filled_store();
        store.clear_history(false).expect("clear");
        assert_eq!(store.generation_count().expect("count"), 0);
        assert!(store.memory().expect("memory").is_empty());
        assert_eq!(store.current().expect("current"), None);
        assert!(store.taste_rows().expect("taste").is_empty());
        assert!(store.hidden_ids().expect("hidden").is_empty());
        // Money spent this month still counts against the budget.
        assert_eq!(store.spend("2026-10").expect("spend"), (100_000, 1));
        // What the person set up stays.
        assert_eq!(keyword_texts(&store), ["fog"]);
        assert_eq!(store.settings().expect("settings"), custom_settings());
        assert_eq!(store.state_get("consecutive_failures").expect("state"), Some(serde_json::json!(1)));
    }

    // ── Taste and spend ─────────────────────────────────────────────────────────────────────

    #[test]
    fn taste_rows_upsert_by_feature_and_reset() {
        let mut store = store();
        assert!(store.taste_rows().expect("empty").is_empty());
        let row = |feature: &str, likes: f64, dislikes: f64, updated_at: i64| TasteRow {
            feature: feature.to_string(),
            likes,
            dislikes,
            updated_at,
        };
        store.save_taste_rows(&[row("fog", 1.0, 0.0, T0), row("neon", 0.0, 2.5, T0)]).expect("save");
        store.save_taste_rows(&[row("fog", 1.75, 0.5, T0 + DAY), row("brume marine", 0.25, 0.0, T0)]).expect("upsert");
        assert_eq!(
            store.taste_rows().expect("rows"),
            vec![row("brume marine", 0.25, 0.0, T0), row("fog", 1.75, 0.5, T0 + DAY), row("neon", 0.0, 2.5, T0)]
        );
        store.save_taste_rows(&[]).expect("nothing to save");
        store.reset_taste().expect("reset");
        assert!(store.taste_rows().expect("after reset").is_empty());
    }

    #[test]
    fn spend_accumulates_per_month() {
        let mut store = store();
        assert_eq!(store.spend("2026-10").expect("none"), (0, 0));
        store.add_spend("2026-10", 26_000, 1).expect("first");
        store.add_spend("2026-10", 100_000, 1).expect("second");
        store.add_spend("2026-11", 151_000, 1).expect("next month");
        store.add_spend("2026-10", 0, 0).expect("local provider");
        assert_eq!(store.spend("2026-10").expect("october"), (126_000, 2));
        assert_eq!(store.spend("2026-11").expect("november"), (151_000, 1));
        assert_eq!(store.spend("2026-12").expect("december"), (0, 0));
    }

    #[test]
    fn spend_rejects_malformed_months_and_unstorable_amounts() {
        let mut store = store();
        for month in ["2026-13", "2026-00", "2026-1", "26-10", "2026/10", "２０２６-10", ""] {
            assert!(matches!(store.add_spend(month, 1, 1), Err(AutoPaperError::InvalidInput { .. })), "{month}");
            assert!(matches!(store.spend(month), Err(AutoPaperError::InvalidInput { .. })), "{month}");
        }
        assert!(matches!(store.add_spend("2026-10", u64::MAX, 1), Err(AutoPaperError::InvalidInput { .. })));
        assert_eq!(store.spend("2026-10").expect("untouched"), (0, 0));
    }

    // ── Unicode ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn unicode_text_survives_every_table() {
        let mut store = store();
        for text in ["海の霧", "🌧 rain", "שקיעה", "Ŝtorm über Fjord"] {
            store.upsert_keyword(text, KeywordWeight::Maybe, T0).expect("keyword");
        }
        assert_eq!(keyword_texts(&store), ["海の霧", "🌧 rain", "שקיעה", "Ŝtorm über Fjord"]);

        let mut stored = with(made("ü-1", 0), |g| {
            g.concept.title = "Mer d’Iroise — brume".into();
            g.concept.summary = "غروب فوق البحر, e\u{301}cume et 霧. 🌊".into();
            g.concept.elements = vec!["phare".into(), "灯台".into()];
            g.echo_note = Some("Echo of “海の霧” (März 2024): nach dem Sturm".into());
            g.keywords = vec![KeywordSnapshot { text: "海の霧".into(), weight: KeywordWeight::Must }];
            g.error = Some("Fehler: Zeitüberschreitung".into());
        });
        stored.embedding_model = "bge-small-en-v1.5".into();
        store.insert_generation(&stored).expect("insert");
        assert_eq!(store.generation("ü-1").expect("read"), Some(stored.clone()));
        assert_eq!(store.memory().expect("memory")[0].title, "Mer d’Iroise — brume");
        assert_eq!(store.recent_summaries(1).expect("summaries"), [stored.generation.concept.summary.clone()]);
        store
            .save_taste_rows(&[TasteRow { feature: "brume marine".into(), likes: 1.0, dislikes: 0.0, updated_at: T0 }])
            .expect("taste");
        assert_eq!(store.taste_rows().expect("taste")[0].feature, "brume marine");
    }

    // ── Moods ───────────────────────────────────────────────────────────────────────────────

    /// A database as the second build left it (migrations 1–2): keywords without moods, a saved Surprise, and
    /// two wallpapers.
    fn version_2(path: &Path, keywords: &[(&str, &str)], surprise: Option<&str>) {
        let mut conn = Connection::open(path).expect("open");
        migrate(&mut conn, &MIGRATIONS[..2]).expect("version 2");
        for (position, (text, weight)) in keywords.iter().enumerate() {
            conn.execute(
                "INSERT INTO keywords (id, text, weight, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![format!("k{position}"), text, weight, position as i64, T0 + position as i64],
            )
            .expect("v2 keyword");
        }
        if let Some(surprise) = surprise {
            conn.execute("INSERT INTO settings (key, value) VALUES ('surprise', ?1)", [surprise]).expect("v2 surprise");
        }
        let v2 = Store { conn };
        for stored in [made("a", 0), failed("x", 1)] {
            // Inserted as the old build did: no mood_id column yet.
            let g = &stored.generation;
            v2.conn
                .execute(
                    "INSERT INTO generations (id, created_at, \"trigger\", status, title, summary, concept_json, prompt,
                     text_provider, text_model, image_provider, image_model, width, height, embedding, embedding_model,
                     surprise, keywords_json) VALUES (?1, ?2, 'scheduled', ?3, ?4, 'S', '{}', 'p', 'demo', 'demo', 'demo',
                     'demo', 1, 1, x'', 'm', 0.3, '[]')",
                    params![g.id, g.created_at, g.status, g.concept.title],
                )
                .expect("v2 generation");
        }
    }

    #[test]
    fn version_2_keywords_and_surprise_become_the_first_mood() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("autopaper.sqlite3");
        version_2(&path, &[("people", "avoid"), ("rain", "must"), ("beach", "maybe"), ("night", "must")], Some("0.6"));
        let store = Store::open(&path).expect("migrate");
        assert_eq!(pragma::<i64>(&store, "user_version"), MIGRATIONS.len() as i64);
        let moods = store.moods().expect("moods");
        assert_eq!(moods.len(), 1);
        let mood = &moods[0];
        assert_eq!((mood.name.as_str(), mood.position, mood.active), ("Rain, Beach", 0, true), "named after the first two that aren't Avoids");
        assert!((mood.surprise - 0.6).abs() < 1e-6);
        assert_eq!(mood.created_at, T0, "as old as its oldest keyword");
        let texts: Vec<(&str, KeywordWeight)> = mood.keywords.iter().map(|k| (k.text.as_str(), k.weight)).collect();
        assert_eq!(
            texts,
            [("people", KeywordWeight::Avoid), ("rain", KeywordWeight::Must), ("beach", KeywordWeight::Maybe), ("night", KeywordWeight::Must)]
        );
        assert_eq!(mood.keywords[1].id, "k1", "keywords keep their ids");
        assert_eq!(store.keywords().expect("active keywords"), mood.keywords);
        assert!((store.settings().expect("settings").surprise - 0.6).abs() < 1e-6, "Surprise is the mood's now");
        // Every wallpaper so far was made under it.
        let a = store.generation("a").expect("read").expect("kept").generation;
        assert_eq!((a.mood_id.as_deref(), a.mood_name.as_deref()), (Some(mood.id.as_str()), Some("Rain, Beach")));
        assert_eq!(store.generation("x").expect("read").expect("kept").generation.mood_id.as_deref(), Some(mood.id.as_str()));
        assert_eq!(store.memory().expect("memory")[0].mood_id.as_deref(), Some(mood.id.as_str()));

        // Without keywords (or a saved Surprise): "My mood", the default Surprise.
        let empty = dir.path().join("empty.sqlite3");
        version_2(&empty, &[], None);
        let store = Store::open(&empty).expect("migrate");
        let mood = store.active_mood().expect("mood");
        assert_eq!((mood.name.as_str(), mood.surprise), ("My mood", Settings::default().surprise));
        // A new database gets one too.
        assert_eq!(self::store().moods().expect("moods").iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["My mood"]);
    }

    #[test]
    fn moods_are_created_renamed_moved_and_deleted() {
        let mut store = store();
        let first = store.active_mood().expect("first");
        store.upsert_keyword("rain", KeywordWeight::Must, T0).expect("keyword");
        store.upsert_keyword("beach", KeywordWeight::Maybe, T0).expect("keyword");
        store.set_mood_surprise(&first.id, 0.7).expect("surprise");

        // Names: trimmed, unique without regard to case, 1–40 characters.
        let reason = |result: Result<Mood>| match result {
            Err(AutoPaperError::InvalidInput { reason, .. }) => reason,
            other => panic!("{other:?}"),
        };
        assert_eq!(reason(store.create_mood("  ", None, T0)), InvalidInputReason::MoodNameEmpty);
        assert_eq!(reason(store.create_mood(&"x".repeat(41), None, T0)), InvalidInputReason::MoodNameTooLong);
        assert_eq!(reason(store.create_mood(" my MOOD ", None, T0)), InvalidInputReason::DuplicateMoodName);
        assert!(matches!(store.create_mood("Copy", Some("nope"), T0), Err(AutoPaperError::NotFound)));

        let snow = store.create_mood("  Snowy   peaks ", None, T0 + 5).expect("new");
        assert_eq!((snow.name.as_str(), snow.position, snow.active), ("Snowy peaks", 1, false), "not made active");
        assert!(snow.keywords.is_empty());
        assert_eq!(snow.surprise, Settings::default().surprise);
        let copy = store.create_mood("Rainy beach", Some(&first.id), T0 + 6).expect("duplicate");
        let words = |mood: &Mood| mood.keywords.iter().map(|k| (k.text.clone(), k.weight, k.position)).collect::<Vec<_>>();
        assert_eq!(words(&copy), words(&store.mood(&first.id).expect("read").expect("first")), "same keywords, same order");
        assert!(copy.keywords.iter().all(|k| first.keywords.iter().all(|o| o.id != k.id)), "new ids");
        assert!((copy.surprise - 0.7).abs() < 1e-6, "and its Surprise");

        assert_eq!(reason(store.rename_mood(&snow.id, "rainy BEACH")), InvalidInputReason::DuplicateMoodName);
        assert_eq!(store.rename_mood(&copy.id, "RAINY beach").expect("own name, new case").name, "RAINY beach");
        assert!(matches!(store.rename_mood("nope", "x"), Err(AutoPaperError::NotFound)));

        store.move_mood(&copy.id, 0).expect("move");
        let order = |store: &Store| store.moods().expect("moods").into_iter().map(|m| (m.name, m.position)).collect::<Vec<_>>();
        assert_eq!(order(&store), [("RAINY beach".to_string(), 0), ("My mood".into(), 1), ("Snowy peaks".into(), 2)]);
        store.move_mood(&copy.id, 99).expect("clamped");
        assert_eq!(order(&store)[2].0, "RAINY beach");

        // Deleting the active mood makes the next one active (the one before when it was last).
        store.set_active_mood(&snow.id).expect("use");
        assert!(store.moods().expect("moods").iter().filter(|m| m.active).map(|m| m.id.clone()).eq([snow.id.clone()]));
        store.delete_mood(&snow.id).expect("delete active");
        assert_eq!(store.active_mood().expect("active").id, copy.id, "the next one");
        store.delete_mood(&copy.id).expect("delete active, last in the list");
        assert_eq!(store.active_mood().expect("active").id, first.id, "the one before");
        assert_eq!(order(&store), [("My mood".to_string(), 0)]);
        assert_eq!(reason(store.delete_mood(&first.id).map(|()| first.clone())), InvalidInputReason::LastMood);
        let count = |store: &Store| -> u32 { store.conn.query_row("SELECT COUNT(*) FROM keywords", [], |row| row.get(0)).expect("count") };
        assert_eq!(count(&store), 2, "a deleted mood's keywords go with it (the copy's two did)");
        // The active mood first in the list: the one after it.
        let second = store.create_mood("Second", None, T0).expect("second");
        store.delete_mood(&first.id).expect("delete the first, active");
        assert_eq!(store.active_mood().expect("active").id, second.id);
        assert_eq!(store.active_mood().expect("active").position, 0);
        assert!(matches!(store.delete_mood("nope"), Err(AutoPaperError::NotFound)));
        assert!(matches!(store.set_active_mood("nope"), Err(AutoPaperError::NotFound)));
        assert_eq!(count(&store), 0, "and the first's");
    }

    #[test]
    fn keywords_belong_to_their_mood() {
        let mut store = store();
        let first = store.active_mood_id().expect("first");
        let other = store.create_mood("Other", None, T0).expect("other").id;
        store.upsert_keyword("rain", KeywordWeight::Must, T0).expect("active");
        // The same text in another mood is another keyword.
        let theirs = store.upsert_keyword_in(&other, "Rain", KeywordWeight::Avoid, T0).expect("other mood");
        store.upsert_keyword_in(&other, "fog", KeywordWeight::Maybe, T0).expect("other mood");
        assert_eq!(keyword_texts(&store), ["rain"], "keywords() is the active mood's");
        let in_other = store.mood_keywords(&other).expect("listed");
        assert_eq!(in_other.iter().map(|k| (k.text.as_str(), k.position)).collect::<Vec<_>>(), [("Rain", 0), ("fog", 1)]);
        assert!(matches!(store.mood_keywords("nope"), Err(AutoPaperError::NotFound)));
        assert!(matches!(store.upsert_keyword_in("nope", "x", KeywordWeight::Must, T0), Err(AutoPaperError::NotFound)));

        // Id-based changes work on any mood's keyword, within that mood.
        assert!(matches!(
            store.rename_keyword(&theirs.id, "FOG"),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::DuplicateKeyword, .. })
        ));
        store.rename_keyword(&theirs.id, "drizzle").expect("rename in its mood");
        store.move_keyword(&theirs.id, 5).expect("move in its mood");
        assert_eq!(store.mood_keywords(&other).expect("listed").iter().map(|k| k.text.as_str()).collect::<Vec<_>>(), ["fog", "drizzle"]);
        store.set_keyword_weight(&theirs.id, KeywordWeight::Must).expect("weight");
        store.delete_keyword(&theirs.id).expect("delete");
        assert_eq!(store.mood_keywords(&other).expect("listed")[0].position, 0, "renumbered");
        assert_eq!(keyword_texts(&store), ["rain"], "the active mood untouched");

        // Each mood has its own 64.
        for n in 0..Keyword::MAX_COUNT - 1 {
            store.upsert_keyword(&format!("k{n}"), KeywordWeight::Maybe, T0).expect("fits");
        }
        assert!(matches!(
            store.upsert_keyword("one more", KeywordWeight::Maybe, T0),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::TooManyKeywords, .. })
        ));
        store.upsert_keyword_in(&other, "still room", KeywordWeight::Maybe, T0).expect("another mood's own limit");
        assert_eq!(store.mood_keywords(&first).expect("listed").len(), Keyword::MAX_COUNT);
    }

    #[test]
    fn surprise_is_the_active_moods() {
        let mut store = store();
        let first = store.active_mood_id().expect("first");
        store.save_settings(&Settings { surprise: 0.9, ..Settings::default() }).expect("save");
        let other = store.create_mood("Calm", None, T0).expect("other").id;
        store.set_active_mood(&other).expect("use");
        assert_eq!(store.settings().expect("read").surprise, Settings::default().surprise, "the new mood's");
        store.save_settings(&Settings { surprise: 0.1, ..Settings::default() }).expect("save");
        assert!((store.mood(&first).expect("read").expect("first").surprise - 0.9).abs() < 1e-6, "the first keeps its own");
        assert!((store.mood(&other).expect("read").expect("other").surprise - 0.1).abs() < 1e-6);
        let row = settings_rows(&store).into_iter().find(|(key, _)| key == "surprise").map(|(_, value)| value);
        let row: f32 = serde_json::from_str(&row.expect("a settings row")).expect("a number");
        assert!((row - 0.1).abs() < 1e-6, "and in the settings row, for older builds");
        store.set_active_mood(&first).expect("back");
        assert!((store.settings().expect("read").surprise - 0.9).abs() < 1e-6);
    }

    #[test]
    fn history_filters_by_mood_and_names_it() {
        let mut store = store();
        let rain = store.active_mood_id().expect("first");
        let snow = store.create_mood("Snow", None, T0).expect("snow").id;
        let under = |stored: StoredGeneration, mood: &str| with(stored, |g| g.mood_id = Some(mood.to_string()));
        insert_all(
            &mut store,
            vec![
                under(made("r1", 0), &rain),
                under(with(made("s1", 1), |g| g.rating = Rating::Liked), &snow),
                under(made("r2", 2), &rain),
                under(with(made("s2", 3), |g| g.echo_of = Some("s1".into())), &snow),
                under(failed("sx", 4), &snow),
            ],
        );
        assert_eq!(ids(&store.history_by_mood(HistoryFilter::All, Some(&snow), 10, 0).expect("snow")), ["s2", "s1"]);
        assert_eq!(ids(&store.history_by_mood(HistoryFilter::All, Some(&rain), 1, 1).expect("paged")), ["r1"]);
        assert_eq!(ids(&store.history_by_mood(HistoryFilter::Liked, Some(&snow), 10, 0).expect("liked")), ["s1"]);
        assert_eq!(ids(&store.history_by_mood(HistoryFilter::Echoes, Some(&rain), 10, 0).expect("none")), Vec::<&str>::new());
        assert_eq!(store.history_by_mood(HistoryFilter::All, None, 10, 0).expect("all"), store.history(HistoryFilter::All, 10, 0).expect("all"));
        assert_eq!(store.recent_least_similar(10, Some(&rain)).expect("rain").len(), 2);

        let named = |store: &Store, id: &str| store.generation(id).expect("read").expect("kept").generation.mood_name;
        assert_eq!(named(&store, "s1").as_deref(), Some("Snow"));
        store.rename_mood(&snow, "Snowfall").expect("rename");
        assert_eq!(named(&store, "s1").as_deref(), Some("Snowfall"), "the mood's name now");
        store.delete_mood(&snow).expect("delete");
        let orphan = store.generation("s1").expect("read").expect("kept").generation;
        assert_eq!((orphan.mood_id.as_deref(), orphan.mood_name), (Some(snow.as_str()), None), "kept, unnamed");
        assert_eq!(ids(&store.history_by_mood(HistoryFilter::All, Some(&snow), 10, 0).expect("still listed")), ["s2", "s1"]);
    }

    #[test]
    fn mood_stats_count_what_history_lists_per_mood() {
        let mut store = store();
        let rain = store.active_mood_id().expect("first");
        let snow = store.create_mood("Snow", None, T0).expect("snow").id;
        let calm = store.create_mood("Calm", None, T0).expect("calm").id;
        let gone = store.create_mood("Gone", None, T0).expect("gone").id;
        let under = |stored: StoredGeneration, mood: &str| with(stored, |g| g.mood_id = Some(mood.to_string()));
        insert_all(
            &mut store,
            vec![
                under(with(made("r1", 0), |g| g.rating = Rating::Liked), &rain),
                under(with(made("r2", 1), |g| g.rating = Rating::Disliked), &rain),
                under(with(made("r3", 2), |g| g.echo_of = Some("r1".into())), &rain),
                under(with(made("r4", 3), |g| g.rating = Rating::Liked), &rain),
                under(made("r5", 4), &rain),
                under(with(made("s1", 5), |g| g.rating = Rating::Liked), &snow),
                under(failed("sx", 6), &snow),
                under(made("g1", 7), &gone),
                made("before-moods", 8),
            ],
        );
        store.delete_mood(&gone).expect("delete");
        store.move_mood(&calm, 0).expect("move");

        let stats = store.mood_stats(4).expect("stats");
        let order: Vec<&str> = stats.iter().map(|s| s.mood_id.as_str()).collect();
        assert_eq!(order, [calm.as_str(), rain.as_str(), snow.as_str()], "every mood that exists, in the person's order");
        let numbers = |s: &MoodStats| (s.wallpapers, s.liked, s.disliked, s.echoes, s.last_made_at);
        assert_eq!(numbers(&stats[0]), (0, 0, 0, 0, None), "a mood that made nothing is listed at zero");
        assert!(stats[0].latest.is_empty());
        assert_eq!(numbers(&stats[1]), (5, 2, 1, 1, Some(T0 + 4 * DAY)));
        assert_eq!(ids(&stats[1].latest), ["r5", "r4", "r3", "r2"], "the newest four, newest first");
        assert_eq!(stats[1].latest[0].mood_name.as_deref(), Some("My mood"));
        assert_eq!(numbers(&stats[2]), (1, 1, 0, 0, Some(T0 + 5 * DAY)), "a failed one isn't counted");
        assert_eq!(ids(&stats[2].latest), ["s1"]);
        assert_eq!(
            stats[1].latest,
            store.history_by_mood(HistoryFilter::All, Some(&rain), 4, 0).expect("page"),
            "the same records History lists"
        );

        store.clear_history(true).expect("clear");
        let cleared = store.mood_stats(4).expect("stats");
        assert!(cleared.iter().all(|s| numbers(s) == (0, 0, 0, 0, None) && s.latest.is_empty()), "nothing left in History");
    }

    #[test]
    fn activity_counts_per_day_and_mood() {
        let mut store = store();
        let rain = store.active_mood_id().expect("first");
        let snow = store.create_mood("Snow", None, T0).expect("snow").id;
        let gone = store.create_mood("Gone", None, T0).expect("gone").id;
        let at = |id: &str, seconds: i64, mood: &str| with(made(id, 0), |g| {
            g.created_at = T0 + seconds;
            g.mood_id = Some(mood.to_string());
        });
        insert_all(
            &mut store,
            vec![
                at("before", -1, &rain),
                at("s1", 0, &snow),
                at("r1", 10, &rain),
                at("r2", DAY - 1, &rain),
                at("g1", DAY - 1, &gone),
                with(failed("x", 0), |g| g.mood_id = Some(rain.clone())),
                at("s2", DAY, &snow),
                at("r3", DAY + 5, &rain),
                at("after", 3 * DAY, &rain),
            ],
        );
        store.delete_mood(&gone).expect("delete");
        store.move_mood(&snow, 0).expect("move");
        // Three days, the middle one 25 hours long (a daylight-saving change); nothing on the last.
        let bounds = [T0, T0 + DAY, T0 + 2 * DAY + 3_600, T0 + 3 * DAY];
        let counts = store.activity(&bounds).expect("activity");
        let rows: Vec<(i64, Option<&str>, u32)> =
            counts.iter().map(|c| (c.day_start, c.mood_id.as_deref(), c.count)).collect();
        assert_eq!(
            rows,
            [
                (T0, Some(snow.as_str()), 1),
                (T0, Some(rain.as_str()), 2),
                (T0, None, 1),
                (T0 + DAY, Some(snow.as_str()), 1),
                (T0 + DAY, Some(rain.as_str()), 1),
            ],
            "by day, then in the moods' order, a deleted mood's last; failed ones and ones outside the days left out"
        );

        assert!(store.activity(&[T0]).is_err(), "one bound is no day");
        assert!(store.activity(&[T0, T0]).is_err(), "bounds must ascend");
        assert!(store.activity(&[T0 + DAY, T0]).is_err());
        let too_many: Vec<i64> = (0..=DayCount::MAX_DAYS as i64 + 1).map(|day| T0 + day * DAY).collect();
        assert!(store.activity(&too_many).is_err());
        assert!(store.activity(&too_many[1..]).is_ok(), "the most days it takes");
    }

    #[test]
    fn keywords_an_older_build_wrote_join_the_active_mood() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("autopaper.sqlite3");
        {
            let mut store = Store::open(&path).expect("open");
            store.upsert_keyword("rain", KeywordWeight::Must, T0).expect("keyword");
            // An older build (no moods) adds keywords after a downgrade: no mood_id.
            for (id, text) in [("old-1", "RAIN"), ("old-2", "harbour")] {
                store
                    .conn
                    .execute(
                        "INSERT INTO keywords (id, text, weight, position, created_at) VALUES (?1, ?2, 'maybe', 7, ?3)",
                        params![id, text, T0],
                    )
                    .expect("older build's keyword");
            }
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(keyword_texts(&store), ["rain", "harbour"], "its duplicate is dropped");
        assert_contiguous(&store);
        // A database whose moods were all removed (another build) gets one back.
        store.conn.execute_batch("PRAGMA foreign_keys = OFF; DELETE FROM moods; PRAGMA foreign_keys = ON;").expect("no moods");
        drop(store);
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.active_mood().expect("mood").name, "My mood");
        assert_eq!(keyword_texts(&store), ["rain", "harbour"]);
    }

    #[test]
    fn the_mood_queries_use_their_indexes() {
        let store = store();
        let plan = |sql: &str| -> String {
            let mut statement = store.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).expect("plan");
            let details = statement
                .query_map(params![10, 0, "m"], |row| row.get::<_, String>(3))
                .expect("plan rows")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("plan details");
            details.join("; ")
        };
        let mood = plan(mood_history_query!(""));
        assert!(mood.contains("generations_by_mood") && !mood.contains("TEMP B-TREE"), "{mood}");
        let keywords = plan("SELECT id FROM keywords WHERE mood_id = ?3 ORDER BY position LIMIT ?1 OFFSET ?2");
        assert!(keywords.contains("INDEX"), "{keywords}");
        let timings = plan(
            "SELECT seconds FROM timings WHERE job = ?3 AND provider = ?3 AND origin = ?3 AND model = ?3
             ORDER BY finished_at DESC, id DESC LIMIT ?1 OFFSET ?2",
        );
        assert!(timings.contains("timings_by_key"), "{timings}");
    }

    // ── Timings ─────────────────────────────────────────────────────────────────────────────

    fn timing(model: &str, seconds: f64, finished_at: i64) -> Timing {
        Timing {
            job: ProviderJob::Images,
            provider: ProviderKind::ComfyUi,
            origin: "http://127.0.0.1:8188".into(),
            model: model.into(),
            width: 2048,
            height: 1152,
            steps: Some(8),
            seconds,
            finished_at,
        }
    }

    #[test]
    fn timings_round_trip_newest_first_per_kind_and_keep_the_newest() {
        let mut store = store();
        let key = |t: &Timing| (t.job, t.provider, t.origin.clone(), t.model.clone());
        store.add_timing(&timing("z", 62.5, T0)).expect("add");
        store.add_timing(&timing("z", 70.0, T0 + 10)).expect("add");
        store.add_timing(&Timing { origin: "http://studio.local:8188".into(), ..timing("z", 300.0, T0) }).expect("another server");
        store.add_timing(&Timing { job: ProviderJob::Concepts, provider: ProviderKind::Demo, origin: String::new(), width: 0, height: 0, steps: None, ..timing("demo", 0.2, T0) }).expect("text");
        let found = store.timings(ProviderJob::Images, ProviderKind::ComfyUi, "http://127.0.0.1:8188", "z").expect("read");
        assert_eq!(found, [timing("z", 70.0, T0 + 10), timing("z", 62.5, T0)]);
        assert!(found.iter().all(|t| key(t) == key(&timing("z", 0.0, 0))));
        assert_eq!(store.timings(ProviderJob::Concepts, ProviderKind::Demo, "", "demo").expect("read")[0].steps, None);
        assert!(store.timings(ProviderJob::Images, ProviderKind::ComfyUi, "http://127.0.0.1:8188", "other").expect("read").is_empty());

        for n in 0..(TIMINGS_KEPT as i64 + 10) {
            store.add_timing(&timing("q", n as f64, T0 + 100 + n)).expect("add");
        }
        let kept = store.timings(ProviderJob::Images, ProviderKind::ComfyUi, "http://127.0.0.1:8188", "q").expect("read");
        assert_eq!(kept.len(), TIMINGS_KEPT as usize);
        assert_eq!(kept.last().map(|t| t.seconds), Some(10.0), "the oldest ten went");
        let all: u32 = store.conn.query_row("SELECT COUNT(*) FROM timings", [], |row| row.get(0)).expect("count");
        assert_eq!(all, TIMINGS_KEPT + 4, "other kinds untouched");
    }
}
