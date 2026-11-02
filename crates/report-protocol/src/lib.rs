//! Versioned, bounded report data shared by the native launcher and server.
use serde::{Deserialize, Serialize};

pub const API_PATH: &str = "/api/v1/bug-reports";
pub const MAX_BODY: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub request_id: String,
    pub category: String,
    pub title: String,
    pub steps: String,
    pub expected: String,
    pub actual: String,
    pub diagnostics: Diagnostics,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub launcher_version: String,
    pub engine_version: String,
    pub source_sha256: String,
    pub platform: String,
    pub os: String,
    pub graphics: String,
    pub log_excerpt: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Receipt {
    pub ticket_id: String,
    pub status: String,
    pub duplicate: bool,
}

impl Report {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("Unsupported report schema");
        }
        if self.request_id.len() != 32 || !self.request_id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid request ID");
        }
        if ![
            "gameplay",
            "interface",
            "crash",
            "graphics",
            "audio",
            "other",
        ]
        .contains(&self.category.as_str())
        {
            return Err("Invalid report category");
        }
        for (text, min, max, error) in [
            (&self.title, 5, 160, "Title must contain 5–160 bytes"),
            (
                &self.steps,
                10,
                8000,
                "Describe reproduction steps (10–8000 bytes)",
            ),
            (&self.expected, 0, 4000, "Expected result is too long"),
            (&self.actual, 0, 4000, "Actual result is too long"),
            (
                &self.diagnostics.launcher_version,
                1,
                64,
                "Invalid launcher version",
            ),
            (
                &self.diagnostics.engine_version,
                0,
                96,
                "Invalid engine version",
            ),
            (
                &self.diagnostics.source_sha256,
                0,
                64,
                "Invalid source hash",
            ),
            (&self.diagnostics.platform, 1, 64, "Invalid platform"),
            (&self.diagnostics.os, 0, 256, "OS description is too long"),
            (
                &self.diagnostics.graphics,
                0,
                512,
                "Graphics description is too long",
            ),
            (
                &self.diagnostics.log_excerpt,
                0,
                24000,
                "Log excerpt is too long",
            ),
        ] {
            if text.trim().len() < min
                || text.len() > max
                || text
                    .chars()
                    .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
            {
                return Err(error);
            }
        }
        if !self.diagnostics.source_sha256.is_empty()
            && (self.diagnostics.source_sha256.len() != 64
                || !self
                    .diagnostics
                    .source_sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit()))
        {
            return Err("Invalid source hash");
        }
        Ok(())
    }
}
