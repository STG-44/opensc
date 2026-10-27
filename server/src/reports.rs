//! Private SQLite storage. Public HTTP accepts reports and returns receipts only.
use axum::{
    Extension, Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use opensc_report_protocol::{Receipt, Report};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

pub struct Store {
    db: Mutex<Connection>,
    slots: Arc<Semaphore>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS tickets(
                id TEXT PRIMARY KEY DEFAULT ('OSC-' || upper(hex(randomblob(6)))),
                fingerprint TEXT NOT NULL UNIQUE, category TEXT NOT NULL, title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK(status IN ('new','triaged','in_progress','fixed','closed')),
                first_seen INTEGER NOT NULL DEFAULT(unixepoch()), last_seen INTEGER NOT NULL DEFAULT(unixepoch()),
                occurrences INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS submissions(
                request_id TEXT PRIMARY KEY, ticket_id TEXT NOT NULL REFERENCES tickets(id),
                created_at INTEGER NOT NULL DEFAULT(unixepoch()), payload TEXT NOT NULL, payload_sha256 TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS submissions_ticket ON submissions(ticket_id);
            CREATE TABLE IF NOT EXISTS status_history(
                ticket_id TEXT NOT NULL REFERENCES tickets(id), changed_at INTEGER NOT NULL DEFAULT(unixepoch()),
                old_status TEXT NOT NULL, new_status TEXT NOT NULL, note TEXT NOT NULL);
            INSERT OR IGNORE INTO schema_migrations(version) VALUES(1);")?;
        Ok(Self {
            db: Mutex::new(connection),
            slots: Arc::new(Semaphore::new(8)),
        })
    }

    fn accept(&self, report: &Report) -> Result<Receipt, (StatusCode, &'static str)> {
        report
            .validate()
            .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;
        let payload = serde_json::to_string(report).map_err(|_| internal())?;
        let payload_hash = hash(payload.as_bytes());
        let grouping = [
            report.category.as_str(),
            &report.title.to_lowercase(),
            report.actual.as_str(),
            report.diagnostics.engine_version.as_str(),
            report.diagnostics.source_sha256.as_str(),
        ]
        .join("\n");
        let fingerprint = hash(grouping.as_bytes());
        let mut db = self.db.lock().map_err(|_| internal())?;
        let tx = db.transaction().map_err(db_error)?;
        let existing: Option<(String, String, String)> = tx.query_row(
            "SELECT s.ticket_id,t.status,s.payload_sha256 FROM submissions s JOIN tickets t ON t.id=s.ticket_id WHERE request_id=?1",
            [&report.request_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(db_error)?;
        if let Some((ticket_id, status, hash)) = existing {
            if hash != payload_hash {
                return Err((
                    StatusCode::CONFLICT,
                    "Request ID already belongs to another report",
                ));
            }
            return Ok(Receipt {
                ticket_id,
                status,
                duplicate: true,
            });
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM submissions", [], |r| r.get(0))
            .map_err(db_error)?;
        if count >= 10000 {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Report storage is full; please retry later",
            ));
        }
        let duplicate: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tickets WHERE fingerprint=?1)",
                [&fingerprint],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        tx.execute("INSERT INTO tickets(fingerprint,category,title) VALUES(?1,?2,?3) ON CONFLICT(fingerprint) DO UPDATE SET last_seen=unixepoch()",
                   params![fingerprint, report.category, report.title]).map_err(db_error)?;
        let (ticket_id, status): (String, String) = tx
            .query_row(
                "SELECT id,status FROM tickets WHERE fingerprint=?1",
                [&fingerprint],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_error)?;
        tx.execute("INSERT INTO submissions(request_id,ticket_id,payload,payload_sha256) VALUES(?1,?2,?3,?4)",
                   params![report.request_id, ticket_id, payload, payload_hash]).map_err(db_error)?;
        tx.execute(
            "UPDATE tickets SET occurrences=occurrences+1,last_seen=unixepoch() WHERE id=?1",
            [&ticket_id],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(Receipt {
            ticket_id,
            status,
            duplicate,
        })
    }

    pub fn list(&self) -> rusqlite::Result<serde_json::Value> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut query = db.prepare("SELECT id,status,category,title,occurrences,first_seen,last_seen FROM tickets ORDER BY last_seen DESC LIMIT 200")?;
        let rows = query.query_map([], |r| Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"occurrences":r.get::<_,i64>(4)?,"first_seen":r.get::<_,i64>(5)?,"last_seen":r.get::<_,i64>(6)?})))?;
        Ok(serde_json::Value::Array(
            rows.collect::<rusqlite::Result<Vec<_>>>()?,
        ))
    }

    pub fn show(&self, ticket: &str) -> rusqlite::Result<serde_json::Value> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut query = db.prepare(
            "SELECT payload FROM submissions WHERE ticket_id=?1 ORDER BY created_at DESC LIMIT 50",
        )?;
        let rows = query.query_map([ticket], |r| r.get::<_, String>(0))?;
        let mut reports = Vec::new();
        for row in rows {
            reports.push(
                serde_json::from_str::<serde_json::Value>(&row?).unwrap_or(serde_json::Value::Null),
            );
        }
        Ok(serde_json::Value::Array(reports))
    }

    pub fn set_status(&self, ticket: &str, status: &str, note: &str) -> Result<(), String> {
        if !["new", "triaged", "in_progress", "fixed", "closed"].contains(&status)
            || note.len() > 2000
        {
            return Err("Invalid status or note".into());
        }
        let mut db = self.db.lock().map_err(|e| e.to_string())?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let old: String = tx
            .query_row("SELECT status FROM tickets WHERE id=?1", [ticket], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE tickets SET status=?1 WHERE id=?2",
            params![status, ticket],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO status_history(ticket_id,old_status,new_status,note) VALUES(?1,?2,?3,?4)",
            params![ticket, old, status, note],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn backup(&self, destination: &Path) -> rusqlite::Result<()> {
        self.db.lock().unwrap_or_else(|e| e.into_inner()).backup(
            rusqlite::MAIN_DB,
            destination,
            rusqlite::backup::NO_PROGRESS,
        )
    }
}

fn internal() -> (StatusCode, &'static str) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Report storage unavailable; please retry later",
    )
}
fn db_error(error: rusqlite::Error) -> (StatusCode, &'static str) {
    eprintln!("Report database error: {error}");
    internal()
}

pub async fn submit(
    Extension(store): Extension<Arc<Store>>,
    Json(report): Json<Report>,
) -> Response {
    if let Err(error) = report.validate() {
        return (StatusCode::UNPROCESSABLE_ENTITY, error).into_response();
    }
    let Ok(permit) = store.slots.clone().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Report service busy; please retry later",
        )
            .into_response();
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        store.accept(&report)
    })
    .await
    {
        Ok(Ok(receipt)) => (StatusCode::CREATED, Json(receipt)).into_response(),
        Ok(Err((status, message))) => (status, message).into_response(),
        Err(error) => {
            eprintln!("Report worker failed: {error}");
            internal().into_response()
        }
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
