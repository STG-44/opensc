//! Профиль внешности персонажа. Атрибуты, навыки и экипировка сохраняются для будущей логики.
use crate::{ConfigLine, Error, Result, config_lines, normalize_path};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct CharacterBody {
    pub kind: String,
    pub body_model: String,
    pub body_map: String,
    pub head_model: String,
    pub head_map: String,
    pub pack: Option<(String, String)>,
    pub extra: Vec<ConfigLine>,
}
#[derive(Debug)]
pub struct CharacterProfile {
    pub id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub initial_body_index: usize,
    pub bodies: Vec<CharacterBody>,
    pub equipment: Vec<ConfigLine>,
    pub properties: Vec<ConfigLine>,
}

impl CharacterProfile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut result = Self {
            id: String::new(),
            first_name: None,
            last_name: None,
            initial_body_index: 0,
            bodies: Vec::new(),
            equipment: Vec::new(),
            properties: Vec::new(),
        };
        let mut fields: Option<BTreeMap<String, String>> = None;
        let mut extra = Vec::new();
        let mut initial = None;
        let mut seen = BTreeSet::new();
        let mut began = false;
        let mut ended = false;
        for line in config_lines(bytes)? {
            let w = &line.words;
            let invalid = || {
                Error::Invalid(format!(
                    "профиль персонажа: строка {} ({})",
                    line.number, w[0]
                ))
            };
            match w[0].as_str() {
                "CHARACTER_BEGIN" => {
                    if began || w.len() != 1 {
                        return Err(invalid());
                    }
                    began = true;
                    continue;
                }
                "CHARACTER_END" => {
                    if !began || ended || fields.is_some() || w.len() != 1 {
                        return Err(invalid());
                    }
                    ended = true;
                    continue;
                }
                _ if !began || ended => return Err(invalid()),
                "BODY_DEFINITION" => {
                    if fields.is_some() || w.len() != 1 || result.bodies.len() >= 256 {
                        return Err(invalid());
                    }
                    fields = Some(BTreeMap::new());
                }
                "BODY_DEFINITION_END" => {
                    if w.len() != 1 {
                        return Err(invalid());
                    }
                    let mut f = fields.take().ok_or_else(invalid)?;
                    let mut required = |key| f.remove(key).ok_or_else(invalid);
                    let kind = required("BODY_TYPE")?;
                    let body_model = normalize_path(&required("BODY_MODEL")?)?;
                    let body_map = normalize_path(&required("BODY_MAP")?)?;
                    let head_model = normalize_path(&required("HEAD_MODEL")?)?;
                    let head_map = normalize_path(&required("HEAD_MAP")?)?;
                    let pack = match (f.remove("PACK_MODEL"), f.remove("PACK_MAP")) {
                        (None, None) => None,
                        (Some(model), Some(map)) => {
                            Some((normalize_path(&model)?, normalize_path(&map)?))
                        }
                        _ => return Err(invalid()),
                    };
                    if !seen.insert(kind.to_ascii_lowercase()) {
                        return Err(invalid());
                    }
                    result.bodies.push(CharacterBody {
                        kind,
                        body_model,
                        body_map,
                        head_model,
                        head_map,
                        pack,
                        extra: std::mem::take(&mut extra),
                    });
                }
                "BODY_TYPE" | "BODY_MODEL" | "BODY_MAP" | "HEAD_MODEL" | "HEAD_MAP"
                | "PACK_MODEL" | "PACK_MAP" => {
                    if w.len() != 2 || w[1].is_empty() {
                        return Err(invalid());
                    }
                    let f = fields.as_mut().ok_or_else(invalid)?;
                    if f.insert(w[0].clone(), w[1].clone()).is_some() {
                        return Err(invalid());
                    }
                }
                _ if fields.is_some() => extra.push(line),
                "CHARACTER" | "FIRSTNAME" | "LASTNAME" | "BODY_INITIAL" => {
                    if w.len() != 2 || w[1].is_empty() {
                        return Err(invalid());
                    }
                    match w[0].as_str() {
                        "CHARACTER" => {
                            if !result.id.is_empty() {
                                return Err(invalid());
                            }
                            result.id = w[1].clone();
                        }
                        key => {
                            let slot = match key {
                                "FIRSTNAME" => &mut result.first_name,
                                "LASTNAME" => &mut result.last_name,
                                _ => &mut initial,
                            };
                            if slot.replace(w[1].clone()).is_some() {
                                return Err(invalid());
                            }
                        }
                    }
                }
                "GIVE_OBJECT" => {
                    if w.len() < 2 || w[1].is_empty() {
                        return Err(invalid());
                    }
                    result.equipment.push(line);
                }
                _ => result.properties.push(line),
            }
        }
        if !ended || fields.is_some() || result.id.is_empty() || result.bodies.is_empty() {
            return Err(Error::Invalid("незавершённый профиль персонажа".into()));
        }
        if let Some(kind) = initial {
            result.initial_body_index = result
                .bodies
                .iter()
                .position(|b| b.kind.eq_ignore_ascii_case(&kind))
                .ok_or_else(|| Error::Invalid(format!("неизвестный BODY_INITIAL {kind}")))?;
        }
        Ok(result)
    }
    pub fn initial_body(&self) -> &CharacterBody {
        &self.bodies[self.initial_body_index]
    }
    pub fn display_name(&self) -> String {
        let words: Vec<_> = [self.first_name.as_deref(), self.last_name.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if words.is_empty() {
            self.id.clone()
        } else {
            words.join(" ")
        }
    }
}
