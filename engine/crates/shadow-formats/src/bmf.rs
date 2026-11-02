//! BMF: кадры движения и каналы узлов. Не зависит от Bevy.
use crate::{Error, Result};
use std::collections::BTreeSet;

#[derive(Debug)]
pub struct BmfMotion {
    pub name: String,
    pub frames_per_second: u32,
    pub ticks_per_frame: u32,
    pub stored_frame_millis: u32,
    pub flags: [u32; 2],
    pub bone_ids: Vec<u32>,
    pub frames: Vec<BmfFrame>,
}
#[derive(Debug)]
pub struct BmfFrame {
    /// Положение корня в пространстве клипа; разность кадров задаёт перемещение.
    pub root_translation: [f32; 3],
    pub index: u32,
    pub flags: [u32; 2],
    pub keys: Vec<BmfKey>,
}
#[derive(Debug)]
pub struct BmfKey {
    pub bone_id: u32,
    /// BMF хранит WXYZ, SMF — XYZW. Порядок явно указан для потребителя.
    pub rotation_wxyz: Option<[f32; 4]>,
    pub translation: Option<[f32; 3]>,
}
impl BmfMotion {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 512 * 1024 * 1024 {
            return Err(invalid("файл больше 512 МиБ"));
        }
        let mut reader = Reader { bytes, offset: 0 };
        let name_bytes = reader.take(128)?;
        let name_end = name_bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| invalid("имя без NUL"))?;
        let (name, _, _) = encoding_rs::WINDOWS_1252.decode(&name_bytes[..name_end]);
        if name.is_empty() {
            return Err(invalid("пустое имя"));
        }
        reader.take(8)?; // Сохранённые hash и адрес массива кадров старого процесса.
        let frame_count = reader.u32()? as usize;
        let first_frame = reader.u32()?;
        let last_frame = reader.u32()?;
        let frames_per_second = reader.u32()?;
        let ticks_per_frame = reader.u32()?;
        let stored_frame_millis = reader.u32()?;
        reader.take(4)?; // Сохранённый адрес массива ID: указатель никогда не используется.
        let bone_count = reader.u32()? as usize;
        let flags = [reader.u32()?, reader.u32()?];
        if frame_count == 0
            || frame_count > 1_000_000
            || frame_count > reader.remaining() / 41
            || first_frame != 0
            || last_frame as usize != frame_count - 1
        {
            return Err(invalid("число или диапазон кадров"));
        }
        if bone_count == 0 || bone_count > 16384 || bone_count > reader.remaining() / 4 {
            return Err(invalid("число костей"));
        }
        if frames_per_second == 0 || frames_per_second > 1000 || ticks_per_frame == 0 {
            return Err(invalid("частота кадров"));
        }
        let mut frames = Vec::with_capacity(frame_count);
        let mut total_keys = 0usize;
        for expected_index in 0..frame_count {
            let root_translation = reader.finite()?;
            let index = reader.u32()?;
            reader.take(4)?; // Адрес ключей старого процесса.
            let key_count = reader.u32()? as usize;
            let flags = [reader.u32()?, reader.u32()?];
            if index as usize != expected_index
                || key_count != bone_count
                || key_count > reader.remaining() / 9
            {
                return Err(invalid("индекс кадра или число ключей"));
            }
            total_keys += key_count;
            if total_keys > 2_000_000 {
                return Err(invalid("слишком много ключей"));
            }
            let mut keys = Vec::with_capacity(key_count);
            let mut ids = BTreeSet::new();
            for _ in 0..key_count {
                if reader.u32()? != index {
                    return Err(invalid("индекс ключа"));
                }
                let bone_id = reader.u32()?;
                if !ids.insert(bone_id) {
                    return Err(invalid("повтор ID в кадре"));
                }
                let mask = reader.take(1)?[0];
                if mask & !3 != 0 {
                    return Err(invalid("неизвестная маска каналов"));
                }
                let rotation_wxyz = if mask & 1 != 0 {
                    let q: [f32; 4] = reader.finite()?;
                    let length = q.iter().map(|v| v * v).sum::<f32>();
                    if !length.is_finite() || length <= f32::EPSILON {
                        return Err(invalid("нулевой кватернион"));
                    }
                    Some(q)
                } else {
                    None
                };
                let translation = if mask & 2 != 0 {
                    Some(reader.finite()?)
                } else {
                    None
                };
                keys.push(BmfKey {
                    bone_id,
                    rotation_wxyz,
                    translation,
                });
            }
            frames.push(BmfFrame {
                root_translation,
                index,
                flags,
                keys,
            });
        }
        let mut bone_ids = Vec::with_capacity(bone_count);
        let mut ids = BTreeSet::new();
        for _ in 0..bone_count {
            let id = reader.u32()?;
            if !ids.insert(id) {
                return Err(invalid("повтор ID в таблице костей"));
            }
            bone_ids.push(id);
        }
        if reader.remaining() != 0 {
            return Err(invalid("лишние байты после таблицы костей"));
        }
        if frames
            .iter()
            .any(|frame| frame.keys.iter().any(|key| !ids.contains(&key.bone_id)))
        {
            return Err(invalid("ключ отсутствует в таблице костей"));
        }
        Ok(Self {
            name: name.into_owned(),
            frames_per_second,
            ticks_per_frame,
            stored_frame_millis,
            flags,
            bone_ids,
            frames,
        })
    }
    /// Последний кадр — конец клипа, а не дополнительный интервал воспроизведения.
    pub fn duration_seconds(&self) -> f32 {
        (self.frames.len().saturating_sub(1)) as f32 / self.frames_per_second as f32
    }
}
fn invalid(message: &str) -> Error {
    Error::Invalid(format!("BMF: {message}"))
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if count > self.remaining() {
            return Err(invalid("обрезанный файл"));
        }
        let start = self.offset;
        self.offset += count;
        Ok(&self.bytes[start..self.offset])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    fn finite<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut result = [0.0; N];
        for value in &mut result {
            *value = f32::from_bits(self.u32()?);
        }
        if result.iter().any(|v| !v.is_finite()) {
            return Err(invalid("NaN или бесконечная координата"));
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(b"test");
        for value in [0u32, 0, 1, 0, 0, 10, 480, 100, 0, 1, 1, 1] {
            bytes.extend(value.to_le_bytes());
        }
        for value in [0u32, 0, 0, 0, 0, 1, 0, 0, 0, 55] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.push(3);
        for value in [1.0f32, 0.0, 0.0, 0.0, 8.0, 2.0, -4.0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(55u32.to_le_bytes());
        bytes
    }
    #[test]
    fn reads_optional_channels_and_wxyz() {
        let motion = BmfMotion::from_bytes(&fixture()).unwrap();
        assert_eq!(
            motion.frames[0].keys[0].rotation_wxyz,
            Some([1.0, 0.0, 0.0, 0.0])
        );
        assert_eq!(motion.frames[0].keys[0].translation, Some([8.0, 2.0, -4.0]));
        assert_eq!(motion.duration_seconds(), 0.0);
        let mut bytes = fixture();
        bytes.drain(217..245);
        bytes[216] = 0;
        let key = &BmfMotion::from_bytes(&bytes).unwrap().frames[0].keys[0];
        assert!(key.rotation_wxyz.is_none() && key.translation.is_none());
    }
    #[test]
    fn rejects_every_truncated_prefix_and_trailing_bytes() {
        let bytes = fixture();
        for size in 0..bytes.len() {
            assert!(BmfMotion::from_bytes(&bytes[..size]).is_err(), "{size}");
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(BmfMotion::from_bytes(&extra).is_err());
    }
    #[test]
    fn rejects_invalid_channels_and_floats() {
        for (offset, value) in [
            (216, 4u32),
            (217, 0),
            (217, f32::NAN.to_bits()),
            (233, f32::INFINITY.to_bits()),
        ] {
            let mut bytes = fixture();
            if offset == 216 {
                bytes[offset] = value as u8;
            } else {
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            assert!(BmfMotion::from_bytes(&bytes).is_err());
        }
    }
    #[test]
    fn rejects_inconsistent_counts_indices_and_ids() {
        for (offset, value) in [
            (136, u32::MAX),
            (164, u32::MAX),
            (148, 0),
            (188, 5),
            (208, 5),
            (245, 54),
        ] {
            let mut bytes = fixture();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(BmfMotion::from_bytes(&bytes).is_err());
        }
    }
}
