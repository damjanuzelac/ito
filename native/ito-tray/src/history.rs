//! Dictation history in SQLite, mirroring the Electron app's `interactions`
//! table shape (minus the raw audio blob).

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

const USER_ID: &str = "self-hosted";

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS interactions (
                id TEXT PRIMARY KEY,
                user_id TEXT,
                title TEXT,
                asr_output TEXT,
                llm_output TEXT,
                duration_ms INTEGER DEFAULT 0,
                sample_rate INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT
            );",
        )?;
        Ok(Self { conn })
    }

    /// Records one dictation. `llm_output` is only set for edit mode.
    pub fn insert_interaction(
        &self,
        transcript: &str,
        llm_output: Option<&str>,
        duration_ms: u64,
        sample_rate: u32,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let title: String = transcript.chars().take(50).collect();
        let asr_json = serde_json::json!({ "transcript": transcript }).to_string();
        let llm_json = llm_output.map(|text| serde_json::json!({ "response": text }).to_string());

        self.conn.execute(
            "INSERT INTO interactions
                (id, user_id, title, asr_output, llm_output, duration_ms, sample_rate,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![
                id,
                USER_ID,
                title,
                asr_json,
                llm_json,
                duration_ms,
                sample_rate,
                now
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)] // used in tests
    pub fn interaction_count(&self) -> Result<i64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM interactions WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_count() {
        let dir = std::env::temp_dir().join(format!("ito-tray-test-{}", uuid::Uuid::new_v4()));
        let db_path = dir.join("test.db");
        let history = History::open(&db_path).unwrap();
        assert_eq!(history.interaction_count().unwrap(), 0);

        history
            .insert_interaction("hello world from the test", None, 1234, 16000)
            .unwrap();
        history
            .insert_interaction("hey ito do something", Some("Done."), 2000, 16000)
            .unwrap();
        assert_eq!(history.interaction_count().unwrap(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
