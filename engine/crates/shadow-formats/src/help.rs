//! Текстовые подсказки оригинала; размеры здесь — метаданные, а не разрешение окна.
use crate::{ConfigLine, Error, Result, text::help_config_lines};
use std::collections::BTreeSet;

#[derive(Debug)]
pub struct HelpDefinitions {
    pub entries: Vec<HelpDefinition>,
    /// В исходнике встречается HELP_BODY с одной открывающей кавычкой.
    pub repaired_quote_lines: Vec<usize>,
}

#[derive(Debug)]
pub struct HelpDefinition {
    pub line: usize,
    pub id: String,
    pub context: bool,
    pub pause_game: bool,
    pub position: Option<[i32; 2]>,
    pub dimensions: Option<[u32; 2]>,
    /// Пустые строки сохраняются для разделения абзацев.
    pub body: Vec<ConfigLine>,
    pub metadata: Vec<ConfigLine>,
}

impl HelpDefinitions {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let (lines, repaired_quote_lines) = help_config_lines(bytes)?;
        let mut entries: Vec<HelpDefinition> = Vec::new();
        let mut ids = BTreeSet::new();
        for line in lines {
            let w = &line.words;
            let invalid =
                || Error::Invalid(format!("подсказки: строка {} ({})", line.number, w[0]));
            if w[0] == "HELP_DEF" {
                if w.len() != 1 || entries.len() >= 10_000 {
                    return Err(invalid());
                }
                if entries.last().is_some_and(|e| e.id.is_empty()) {
                    return Err(Error::Invalid("подсказка без HELP_ID".into()));
                }
                entries.push(HelpDefinition {
                    line: line.number,
                    id: String::new(),
                    context: false,
                    pause_game: true,
                    position: None,
                    dimensions: None,
                    body: Vec::new(),
                    metadata: Vec::new(),
                });
                continue;
            }
            let entry = entries.last_mut().ok_or_else(invalid)?;
            match w[0].as_str() {
                "HELP_ID" => {
                    if w.len() != 2
                        || w[1].is_empty()
                        || !entry.id.is_empty()
                        || !ids.insert(w[1].to_ascii_lowercase())
                    {
                        return Err(invalid());
                    }
                    entry.id = w[1].clone();
                }
                "HELP_CONTEXT" | "HELP_DO_NOT_PAUSE_GAME" => {
                    if w.len() != 1 {
                        return Err(invalid());
                    }
                    if w[0] == "HELP_CONTEXT" {
                        entry.context = true;
                    } else {
                        entry.pause_game = false;
                    }
                }
                "HELP_POS" => {
                    if w.len() != 3 || entry.position.is_some() {
                        return Err(invalid());
                    }
                    entry.position = Some([
                        w[1].parse().map_err(|_| invalid())?,
                        w[2].parse().map_err(|_| invalid())?,
                    ]);
                }
                "HELP_DIMS" => {
                    if w.len() != 3 || entry.dimensions.is_some() {
                        return Err(invalid());
                    }
                    let values = [
                        w[1].parse::<u32>().map_err(|_| invalid())?,
                        w[2].parse::<u32>().map_err(|_| invalid())?,
                    ];
                    if values.iter().any(|v| *v == 0 || *v > 65535) {
                        return Err(invalid());
                    }
                    entry.dimensions = Some(values);
                }
                "HELP_BODY" => {
                    if w.len() != 2 {
                        return Err(invalid());
                    }
                    entry.body.push(line);
                }
                _ => entry.metadata.push(line),
            }
        }
        if entries.is_empty() || entries.last().is_some_and(|e| e.id.is_empty()) {
            return Err(Error::Invalid(
                "нет определений подсказок или HELP_ID".into(),
            ));
        }
        Ok(Self {
            entries,
            repaired_quote_lines,
        })
    }

    pub fn get(&self, id: &str) -> Option<&HelpDefinition> {
        self.entries
            .iter()
            .find(|entry| entry.id.eq_ignore_ascii_case(id))
    }
}
