//! Append-only journal - the single source of truth for kernel state.
//!
//! The journal records every state transition as an immutable entry.
//! Entries are ordered by a monotonically increasing sequence number.
//!
//! # On-disk format (SQLite)
//!
//! ```sql
//! CREATE TABLE journal_entries (
//!     id INTEGER PRIMARY KEY AUTOINCREMENT,
//!     sequence INTEGER NOT NULL UNIQUE,
//!     workload_id TEXT NOT NULL,
//!     entry_type TEXT NOT NULL,     -- "INTENT", "TRANSITION", "OBSERVATION", "COMMIT", "ABORT", "COMPENSATE"
//!     object_type TEXT NOT NULL,    -- "Workload", "Model", "Engine", "Device", etc.
//!     object_id TEXT NOT NULL,
//!     previous_phase TEXT,
//!     new_phase TEXT,
//!     generation INTEGER NOT NULL,
//!     payload BLOB NOT NULL,        -- Serialized state as JSON
//!     fencing_token TEXT NOT NULL,  -- For CAS validation
//!     actor TEXT NOT NULL,          -- Who made the change
//!     idempotency_key TEXT,
//!     timestamp_us INTEGER NOT NULL,
//!     checksum BLOB NOT NULL,       -- SHA-256 of every immutable entry field
//!     created_at TEXT NOT NULL DEFAULT (datetime('now'))
//! );
//!
//! CREATE INDEX idx_journal_workload ON journal_entries(workload_id, sequence);
//! CREATE INDEX idx_journal_object ON journal_entries(object_type, object_id);
//! CREATE UNIQUE INDEX idx_journal_idempotency ON journal_entries(idempotency_key)
//!     WHERE idempotency_key IS NOT NULL;
//! ```

use nous_types::error::{ErrorCode, NousError};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

/// A single entry in the state journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub sequence: u64,
    pub workload_id: String,
    pub entry_type: EntryType,
    pub object_type: String,
    pub object_id: String,
    pub previous_phase: Option<String>,
    pub new_phase: String,
    pub generation: u64,
    pub payload: Vec<u8>,
    pub fencing_token: String,
    pub actor: String,
    pub idempotency_key: Option<String>,
    pub timestamp_us: i64,
    pub checksum: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryType {
    Intent,
    Transition,
    Observation,
    Commit,
    Abort,
    Compensate,
}

impl EntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryType::Intent => "INTENT",
            EntryType::Transition => "TRANSITION",
            EntryType::Observation => "OBSERVATION",
            EntryType::Commit => "COMMIT",
            EntryType::Abort => "ABORT",
            EntryType::Compensate => "COMPENSATE",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "INTENT" => Some(EntryType::Intent),
            "TRANSITION" => Some(EntryType::Transition),
            "OBSERVATION" => Some(EntryType::Observation),
            "COMMIT" => Some(EntryType::Commit),
            "ABORT" => Some(EntryType::Abort),
            "COMPENSATE" => Some(EntryType::Compensate),
            _ => None,
        }
    }
}

/// The state journal - append-only, thread-safe.
pub struct Journal {
    conn: Mutex<Connection>,
    next_sequence: Mutex<u64>,
}

/// On-disk journal schema and checksum format.
pub const JOURNAL_FORMAT_VERSION: u32 = 2;

impl Journal {
    /// Open or create a journal at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, NousError> {
        let conn = Connection::open(path).map_err(|e| {
            NousError::new(
                ErrorCode::Internal,
                format!("Failed to open journal: {}", e),
            )
        })?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA trusted_schema=OFF;",
        )
        .map_err(|e| {
            NousError::new(ErrorCode::Internal, format!("Failed to set pragmas: {}", e))
        })?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS journal_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS journal_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sequence INTEGER NOT NULL UNIQUE,
                workload_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                object_type TEXT NOT NULL,
                object_id TEXT NOT NULL,
                previous_phase TEXT,
                new_phase TEXT NOT NULL,
                generation INTEGER NOT NULL,
                payload BLOB NOT NULL,
                fencing_token TEXT NOT NULL,
                actor TEXT NOT NULL,
                idempotency_key TEXT,
                timestamp_us INTEGER NOT NULL,
                checksum BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_journal_workload ON journal_entries(workload_id, sequence);
            CREATE INDEX IF NOT EXISTS idx_journal_object ON journal_entries(object_type, object_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_journal_idempotency ON journal_entries(idempotency_key)
                WHERE idempotency_key IS NOT NULL;"
        ).map_err(|e| NousError::new(ErrorCode::Internal, format!("Failed to create schema: {}", e)))?;

        // Recover the next sequence number
        let max_seq: u64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM journal_entries",
                [],
                |row| row.get(0),
            )
            .map_err(|error| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to recover journal sequence: {error}"),
                )
            })?;
        let format_version: Option<String> = conn
            .query_row(
                "SELECT value FROM journal_metadata WHERE key = 'format_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to read journal format version: {error}"),
                )
            })?;
        match format_version {
            Some(version) if version == JOURNAL_FORMAT_VERSION.to_string() => {}
            Some(version) => {
                return Err(NousError::new(
                    ErrorCode::FailedPrecondition,
                    format!(
                        "Journal format {version} is incompatible with format {JOURNAL_FORMAT_VERSION}"
                    ),
                ));
            }
            None if max_seq == 0 => {
                conn.execute(
                    "INSERT INTO journal_metadata (key, value) VALUES ('format_version', ?1)",
                    [JOURNAL_FORMAT_VERSION.to_string()],
                )
                .map_err(|error| {
                    NousError::new(
                        ErrorCode::Internal,
                        format!("Failed to initialize journal format version: {error}"),
                    )
                })?;
            }
            None => {
                return Err(NousError::new(
                    ErrorCode::FailedPrecondition,
                    "Unversioned non-empty journal requires an explicit migration",
                ));
            }
        }

        Ok(Self {
            conn: Mutex::new(conn),
            next_sequence: Mutex::new(max_seq + 1),
        })
    }

    /// Open an in-memory journal (for testing).
    pub fn open_in_memory() -> Result<Self, NousError> {
        let conn = Connection::open_in_memory().map_err(|e| {
            NousError::new(
                ErrorCode::Internal,
                format!("Failed to open in-memory journal: {}", e),
            )
        })?;

        conn.execute_batch(
            "CREATE TABLE journal_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sequence INTEGER NOT NULL UNIQUE,
                workload_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                object_type TEXT NOT NULL,
                object_id TEXT NOT NULL,
                previous_phase TEXT,
                new_phase TEXT NOT NULL,
                generation INTEGER NOT NULL,
                payload BLOB NOT NULL,
                fencing_token TEXT NOT NULL,
                actor TEXT NOT NULL,
                idempotency_key TEXT,
                timestamp_us INTEGER NOT NULL,
                checksum BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_journal_workload ON journal_entries(workload_id, sequence);
            CREATE UNIQUE INDEX idx_journal_idempotency ON journal_entries(idempotency_key)
                WHERE idempotency_key IS NOT NULL;",
        )
        .map_err(|e| {
            NousError::new(
                ErrorCode::Internal,
                format!("Failed to create schema: {}", e),
            )
        })?;

        Ok(Self {
            conn: Mutex::new(conn),
            next_sequence: Mutex::new(1),
        })
    }

    /// Append an entry to the journal.
    ///
    /// Returns the assigned sequence number.
    /// If an entry with the same idempotency_key already exists, returns its
    /// sequence number instead (idempotent append).
    pub fn append(&self, mut entry: JournalEntry) -> Result<u64, NousError> {
        // Check idempotency
        if let Some(ref key) = entry.idempotency_key {
            if let Some(seq) = self.find_by_idempotency_key(key)? {
                tracing::info!(sequence = seq, idempotency_key = %key, "Journal append: idempotent replay");
                return Ok(seq);
            }
        }

        let mut seq = self.next_sequence.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal sequence lock is poisoned")
        })?;
        entry.sequence = *seq;

        // Compute timestamp if not set
        if entry.timestamp_us == 0 {
            entry.timestamp_us = chrono::Utc::now().timestamp_micros();
        }

        // Protect the complete immutable record, not only its payload.
        entry.checksum = Self::compute_checksum(&entry);

        let conn = self.conn.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal connection lock is poisoned")
        })?;
        conn.execute(
            "INSERT INTO journal_entries
             (sequence, workload_id, entry_type, object_type, object_id,
              previous_phase, new_phase, generation, payload, fencing_token,
              actor, idempotency_key, timestamp_us, checksum)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                entry.sequence,
                entry.workload_id,
                entry.entry_type.as_str(),
                entry.object_type,
                entry.object_id,
                entry.previous_phase,
                entry.new_phase,
                entry.generation,
                entry.payload,
                entry.fencing_token,
                entry.actor,
                entry.idempotency_key,
                entry.timestamp_us,
                entry.checksum,
            ],
        )
        .map_err(|e| {
            // Check for unique constraint violation (idempotency)
            if e.to_string().contains("UNIQUE") && e.to_string().contains("idempotency") {
                NousError::new(ErrorCode::AlreadyExists, "Idempotency key already used")
            } else {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to append journal entry: {}", e),
                )
            }
        })?;

        let assigned = *seq;
        *seq += 1;
        Ok(assigned)
    }

    /// Find an entry by idempotency key.
    fn find_by_idempotency_key(&self, key: &str) -> Result<Option<u64>, NousError> {
        let conn = self.conn.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal connection lock is poisoned")
        })?;
        let result: Option<u64> = conn
            .query_row(
                "SELECT sequence FROM journal_entries WHERE idempotency_key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to query idempotency: {}", e),
                )
            })?;
        Ok(result)
    }

    /// Read all entries for a workload, ordered by sequence.
    pub fn read_workload(&self, workload_id: &str) -> Result<Vec<JournalEntry>, NousError> {
        let conn = self.conn.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal connection lock is poisoned")
        })?;
        let mut stmt = conn
            .prepare(
                "SELECT sequence, workload_id, entry_type, object_type, object_id,
                        previous_phase, new_phase, generation, payload, fencing_token,
                        actor, idempotency_key, timestamp_us, checksum
                 FROM journal_entries
                 WHERE workload_id = ?1
                 ORDER BY sequence ASC",
            )
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to prepare query: {}", e),
                )
            })?;

        let entries = stmt
            .query_map(params![workload_id], decode_entry)
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to query entries: {}", e),
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to collect entries: {}", e),
                )
            })?;

        Ok(entries)
    }

    /// Read all entries from a given sequence number (for watch/resume).
    pub fn read_from(&self, from_sequence: u64) -> Result<Vec<JournalEntry>, NousError> {
        let conn = self.conn.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal connection lock is poisoned")
        })?;
        let mut stmt = conn
            .prepare(
                "SELECT sequence, workload_id, entry_type, object_type, object_id,
                        previous_phase, new_phase, generation, payload, fencing_token,
                        actor, idempotency_key, timestamp_us, checksum
                 FROM journal_entries
                 WHERE sequence >= ?1
                 ORDER BY sequence ASC",
            )
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to prepare query: {}", e),
                )
            })?;

        let entries = stmt
            .query_map(params![from_sequence], decode_entry)
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to query entries: {}", e),
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("Failed to collect entries: {}", e),
                )
            })?;

        Ok(entries)
    }

    /// Get the current maximum sequence number.
    pub fn current_sequence(&self) -> Result<u64, NousError> {
        Ok(*self.next_sequence.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal sequence lock is poisoned")
        })? - 1)
    }

    /// Count entries for one object type without materializing journal payloads.
    pub fn count_object_type(&self, object_type: &str) -> Result<u64, NousError> {
        let conn = self.conn.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal connection lock is poisoned")
        })?;
        conn.query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE object_type = ?1",
            params![object_type],
            |row| row.get(0),
        )
        .map_err(|error| {
            NousError::new(
                ErrorCode::Internal,
                format!("Failed to count journal object type: {error}"),
            )
        })
    }

    /// Verify the integrity of the journal by checking all checksums.
    pub fn verify_integrity(&self) -> Result<Vec<u64>, NousError> {
        let conn = self.conn.lock().map_err(|_| {
            NousError::new(ErrorCode::Internal, "Journal connection lock is poisoned")
        })?;
        let storage_status: String = conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|error| {
                NousError::new(
                    ErrorCode::DataLoss,
                    format!("Journal storage check failed: {error}"),
                )
            })?;
        if storage_status != "ok" {
            return Err(NousError::new(
                ErrorCode::DataLoss,
                format!("Journal storage is corrupt: {storage_status}"),
            ));
        }
        let mut stmt = conn
            .prepare(
                "SELECT sequence, workload_id, entry_type, object_type, object_id,
                        previous_phase, new_phase, generation, payload, fencing_token,
                        actor, idempotency_key, timestamp_us, checksum
                 FROM journal_entries ORDER BY sequence",
            )
            .map_err(|e| {
                NousError::new(ErrorCode::Internal, format!("Failed to prepare: {}", e))
            })?;

        let mut corrupted = Vec::new();
        let rows = stmt
            .query_map([], decode_entry)
            .map_err(|e| NousError::new(ErrorCode::Internal, format!("Failed to query: {}", e)))?;

        for row in rows {
            let entry = row.map_err(|e| {
                NousError::new(ErrorCode::Internal, format!("Failed to read row: {}", e))
            })?;
            let computed = Self::compute_checksum(&entry);
            if computed != entry.checksum {
                corrupted.push(entry.sequence);
            }
        }

        Ok(corrupted)
    }

    /// Compute SHA-256 checksum of every immutable record field.
    fn compute_checksum(entry: &JournalEntry) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(entry.sequence.to_be_bytes());
        hasher.update(entry.workload_id.as_bytes());
        hasher.update(entry.entry_type.as_str().as_bytes());
        hasher.update(entry.object_type.as_bytes());
        hasher.update(entry.object_id.as_bytes());
        if let Some(previous_phase) = &entry.previous_phase {
            hasher.update(previous_phase.as_bytes());
        }
        hasher.update(entry.new_phase.as_bytes());
        hasher.update(entry.generation.to_be_bytes());
        hasher.update(&entry.payload);
        hasher.update(entry.fencing_token.as_bytes());
        hasher.update(entry.actor.as_bytes());
        if let Some(idempotency_key) = &entry.idempotency_key {
            hasher.update(idempotency_key.as_bytes());
        }
        hasher.update(entry.timestamp_us.to_be_bytes());
        hasher.finalize().to_vec()
    }
}

fn decode_entry(row: &Row<'_>) -> rusqlite::Result<JournalEntry> {
    let raw_entry_type: String = row.get(2)?;
    let entry_type = EntryType::parse(&raw_entry_type).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown journal entry type: {raw_entry_type}"),
            )),
        )
    })?;
    Ok(JournalEntry {
        sequence: row.get(0)?,
        workload_id: row.get(1)?,
        entry_type,
        object_type: row.get(3)?,
        object_id: row.get(4)?,
        previous_phase: row.get(5)?,
        new_phase: row.get(6)?,
        generation: row.get(7)?,
        payload: row.get(8)?,
        fencing_token: row.get(9)?,
        actor: row.get(10)?,
        idempotency_key: row.get(11)?,
        timestamp_us: row.get(12)?,
        checksum: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(idempotency_key: Option<&str>) -> JournalEntry {
        JournalEntry {
            sequence: 0, // Will be assigned by journal
            workload_id: "test-workload-1".to_string(),
            entry_type: EntryType::Transition,
            object_type: "Workload".to_string(),
            object_id: "test-workload-1".to_string(),
            previous_phase: Some("CREATED".to_string()),
            new_phase: "VALIDATING".to_string(),
            generation: 1,
            payload: b"{}".to_vec(),
            fencing_token: "token-1".to_string(),
            actor: "test".to_string(),
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            timestamp_us: 0,
            checksum: Vec::new(),
        }
    }

    #[test]
    fn test_append_and_read() {
        let journal = Journal::open_in_memory().unwrap();
        let seq = journal.append(make_entry(None)).unwrap();
        assert_eq!(seq, 1);

        let entries = journal.read_workload("test-workload-1").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].new_phase, "VALIDATING");
    }

    #[test]
    fn file_journal_records_and_reopens_current_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.db");
        let journal = Journal::open(&path).unwrap();
        journal.append(make_entry(Some("format"))).unwrap();
        drop(journal);
        let reopened = Journal::open(path).unwrap();
        assert_eq!(reopened.current_sequence().unwrap(), 1);
    }

    #[test]
    fn test_idempotent_append() {
        let journal = Journal::open_in_memory().unwrap();
        let seq1 = journal.append(make_entry(Some("key-1"))).unwrap();
        let seq2 = journal.append(make_entry(Some("key-1"))).unwrap();
        assert_eq!(seq1, seq2); // Same sequence - idempotent
        assert_eq!(journal.read_workload("test-workload-1").unwrap().len(), 1);
    }

    #[test]
    fn test_read_from_sequence() {
        let journal = Journal::open_in_memory().unwrap();
        journal.append(make_entry(Some("a"))).unwrap();
        journal.append(make_entry(Some("b"))).unwrap();
        journal.append(make_entry(Some("c"))).unwrap();

        let entries = journal.read_from(2).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 2);
    }

    #[test]
    fn test_integrity_check() {
        let journal = Journal::open_in_memory().unwrap();
        journal.append(make_entry(Some("x"))).unwrap();
        let corrupted = journal.verify_integrity().unwrap();
        assert!(corrupted.is_empty());
    }

    #[test]
    fn tampered_record_header_is_detected() {
        let journal = Journal::open_in_memory().unwrap();
        journal.append(make_entry(Some("tamper"))).unwrap();
        journal
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE journal_entries SET new_phase = 'COMMITTED' WHERE sequence = 1",
                [],
            )
            .unwrap();
        assert_eq!(journal.verify_integrity().unwrap(), vec![1]);
    }

    #[test]
    fn concurrent_appends_keep_a_strictly_monotonic_sequence() {
        let journal = std::sync::Arc::new(Journal::open_in_memory().unwrap());
        let mut threads = Vec::new();
        for worker in 0..4 {
            let journal = journal.clone();
            threads.push(std::thread::spawn(move || {
                for index in 0..25 {
                    journal
                        .append(make_entry(Some(&format!("{worker}-{index}"))))
                        .unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let entries = journal.read_from(1).unwrap();
        assert_eq!(entries.len(), 100);
        assert!(entries
            .iter()
            .enumerate()
            .all(|(index, entry)| entry.sequence == index as u64 + 1));
    }
}
