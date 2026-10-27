//! Одноплоскостной PCX с 8-битными индексами: формат карт высот оригинала.
use crate::{Error, Result};
#[derive(Debug)]
pub struct PcxImage {
    pub width: usize,
    pub height: usize,
    /// Строки сверху вниз; padding каждой строки удалён. Это индексы, не RGB.
    pub pixels: Vec<u8>,
}
impl PcxImage {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let invalid = |s: &str| Error::Invalid(format!("PCX: {s}"));
        if bytes.len() < 128 || bytes.len() > 32 * 1024 * 1024 {
            return Err(invalid("неверный размер файла"));
        }
        if bytes[0] != 10 || bytes[1] != 5 || bytes[2] != 1 || bytes[3] != 8 || bytes[65] != 1 {
            return Err(invalid("нужен PCX v5, RLE, 8 бит и одна плоскость"));
        }
        let u16_at = |i| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        let width = u16_at(8)
            .checked_sub(u16_at(4))
            .map(|v| v + 1)
            .ok_or_else(|| invalid("границы X"))?;
        let height = u16_at(10)
            .checked_sub(u16_at(6))
            .map(|v| v + 1)
            .ok_or_else(|| invalid("границы Y"))?;
        let stride = u16_at(66);
        if width > 4096
            || height > 4096
            || stride < width
            || stride > 8192
            || width * height > 4_194_304
        {
            return Err(invalid("слишком большие размеры или неверный stride"));
        }
        let mut offset = 128;
        let mut pixels = Vec::with_capacity(width * height);
        for _ in 0..height {
            let mut column = 0;
            while column < stride {
                let byte = *bytes.get(offset).ok_or_else(|| invalid("обрезан RLE"))?;
                offset += 1;
                let (count, value) = if byte & 0xc0 == 0xc0 {
                    let count = (byte & 0x3f) as usize;
                    let value = *bytes.get(offset).ok_or_else(|| invalid("обрезана серия"))?;
                    offset += 1;
                    (count, value)
                } else {
                    (1, byte)
                };
                if count == 0 || column + count > stride {
                    return Err(invalid("серия выходит за строку или имеет нулевую длину"));
                }
                pixels.extend(std::iter::repeat_n(
                    value,
                    (column + count).min(width).saturating_sub(column),
                ));
                column += count;
            }
        }
        let tail = &bytes[offset..];
        if !(tail.is_empty() || (tail.len() == 769 && tail[0] == 12)) {
            return Err(invalid("неожиданный остаток вместо палитры"));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
}
