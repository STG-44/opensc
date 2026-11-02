//! MTF хранит расстановку объектов. Рельеф и текстуры загружаются отдельно.
use crate::{ConfigLine, Error, Result, config_lines};

#[derive(Debug)]
pub struct MtfMap {
    pub globals: Vec<ConfigLine>,
    pub objects: Vec<MtfObject>,
}
#[derive(Debug)]
pub struct MtfObject {
    pub inventory: bool,
    pub class: String,
    pub model: String,
    pub name: String,
    /// None означает исходное Microsoft `-1.#IND00`; такой объект не рендерится.
    pub position: Option<[f32; 3]>,
    pub rotation: Option<[f32; 3]>,
    pub id: [i32; 2],
    /// Сохраняем дополнительные команды, но не выполняем игровую логику.
    pub properties: Vec<ConfigLine>,
}
impl MtfMap {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut result = Self {
            globals: Vec::new(),
            objects: Vec::new(),
        };
        let mut present = [false; 3];
        for line in config_lines(bytes)? {
            let w = &line.words;
            match w[0].as_str() {
                "OBJECT" | "OBJECT_INVENTORY" => {
                    if !result.objects.is_empty() && !present.into_iter().all(|v| v) {
                        return Err(Error::Invalid(
                            "у объекта отсутствует положение, поворот или ID".into(),
                        ));
                    }
                    if w.len() != 4 || result.objects.len() >= 100_000 {
                        return Err(Error::Invalid(format!(
                            "неверное объявление объекта: строка {}",
                            line.number
                        )));
                    }
                    result.objects.push(MtfObject {
                        inventory: w[0] == "OBJECT_INVENTORY",
                        class: w[1].clone(),
                        model: w[2].clone(),
                        name: w[3].clone(),
                        position: None,
                        rotation: None,
                        id: [0; 2],
                        properties: Vec::new(),
                    });
                    present = [false; 3];
                }
                "OBJECT_POSITION" | "OBJECT_ROTATION" | "OBJECT_ID" => {
                    let object = result.objects.last_mut().ok_or_else(|| {
                        Error::Invalid(format!("свойство без объекта: строка {}", line.number))
                    })?;
                    let slot = match w[0].as_str() {
                        "OBJECT_POSITION" => 0,
                        "OBJECT_ROTATION" => 1,
                        _ => 2,
                    };
                    if present[slot] || w.len() != if slot == 2 { 3 } else { 4 } {
                        return Err(Error::Invalid(format!(
                            "неверное или повторное свойство: строка {}",
                            line.number
                        )));
                    }
                    if slot == 2 {
                        for (out, word) in object.id.iter_mut().zip(&w[1..]) {
                            *out = word.parse().map_err(|_| {
                                Error::Invalid(format!("неверный ID: строка {}", line.number))
                            })?;
                        }
                    } else {
                        let target = if slot == 0 {
                            &mut object.position
                        } else {
                            &mut object.rotation
                        };
                        let mut values = [0.0; 3];
                        let mut invalid_original = false;
                        for (out, word) in values.iter_mut().zip(&w[1..]) {
                            if word == "-1.#IND00" || word == "1.#IND00" {
                                invalid_original = true;
                                continue;
                            }
                            *out = word
                                .parse::<f32>()
                                .ok()
                                .filter(|v| v.is_finite())
                                .ok_or_else(|| {
                                    Error::Invalid(format!(
                                        "неверное число: строка {}",
                                        line.number
                                    ))
                                })?;
                        }
                        *target = if invalid_original { None } else { Some(values) };
                        if invalid_original {
                            object.properties.push(line.clone());
                        }
                    }
                    present[slot] = true;
                }
                key if key.starts_with("OBJECT_") => {
                    result
                        .objects
                        .last_mut()
                        .ok_or_else(|| Error::Invalid("свойство без объекта".into()))?
                        .properties
                        .push(line);
                }
                _ => result.globals.push(line),
            }
        }
        if !result.objects.is_empty() && !present.into_iter().all(|v| v) {
            return Err(Error::Invalid("последний объект не завершён".into()));
        }
        Ok(result)
    }
}
