//! Конфигурации оригинала: Windows-1252, кавычки и комментарий `;` вне кавычек.
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct ConfigLine {
    pub number: usize,
    pub words: Vec<String>,
}

pub fn config_lines(bytes: &[u8]) -> Result<Vec<ConfigLine>> {
    Ok(config_lines_inner(bytes, false)?.0)
}

/// Только HELP_BODY оригинала допускает пропущенную закрывающую кавычку.
/// Общий загрузчик конфигураций остаётся строгим; возвращаем номера исправлений.
pub(crate) fn help_config_lines(bytes: &[u8]) -> Result<(Vec<ConfigLine>, Vec<usize>)> {
    config_lines_inner(bytes, true)
}

fn config_lines_inner(
    bytes: &[u8],
    tolerate_help_body: bool,
) -> Result<(Vec<ConfigLine>, Vec<usize>)> {
    if bytes.len() > 16 * 1024 * 1024 || bytes.contains(&0) {
        return Err(Error::Invalid(
            "текст слишком велик или содержит NUL".into(),
        ));
    }
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    let mut lines = Vec::new();
    let mut repairs = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.len() > 16384 {
            return Err(Error::Invalid(format!(
                "слишком длинная строка {}",
                index + 1
            )));
        }
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        let mut started = false;
        for c in line.chars() {
            if c == '"' {
                quoted = !quoted;
                started = true;
            } else if c == ';' && !quoted {
                break;
            } else if c.is_whitespace() && !quoted {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            } else {
                word.push(c);
                started = true;
            }
        }
        if quoted {
            if tolerate_help_body
                && words == ["HELP_BODY"]
                && line.chars().filter(|c| *c == '"').count() == 1
                && line
                    .trim_start()
                    .strip_prefix("HELP_BODY")
                    .is_some_and(|tail| tail.trim_start().starts_with('"'))
            {
                repairs.push(index + 1);
            } else {
                return Err(Error::Invalid(format!(
                    "незакрытые кавычки: строка {}",
                    index + 1
                )));
            }
        }
        if started {
            words.push(word);
        }
        if !words.is_empty() {
            lines.push(ConfigLine {
                number: index + 1,
                words,
            });
        }
    }
    Ok((lines, repairs))
}
