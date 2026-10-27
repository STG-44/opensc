//! RIFF AVI: независимые MJPEG-кадры и необязательный PCM16LE-звук.
//! Храним сжатый файл один раз, кадры и звук — диапазонами внутри него.
use crate::{Error, Result};
use std::{ops::Range, sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy)]
pub struct AviAudio {
    pub channels: u16,
    pub sample_rate: u32,
    /// Количество отсчётов всех каналов, а не аудиокадров.
    pub samples: usize,
}
impl AviAudio {
    pub fn duration(self) -> Duration {
        Duration::from_secs_f64(
            self.samples as f64 / self.channels as f64 / self.sample_rate as f64,
        )
    }
}
#[derive(Debug)]
pub struct AviMovie {
    bytes: Arc<[u8]>,
    video: Vec<Range<usize>>,
    audio_chunks: Vec<Range<usize>>,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub rate: u32,
    pub audio: Option<AviAudio>,
}
#[derive(Clone)]
struct Chunk {
    tag: [u8; 4],
    data: Range<usize>,
}
struct Stream {
    kind: [u8; 4],
    header: Range<usize>,
    format: Range<usize>,
}
fn invalid(message: &str) -> Error {
    Error::Invalid(format!("AVI: {message}"))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(|| invalid("обрезан заголовок"))?
            .try_into()
            .unwrap(),
    ))
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(|| invalid("обрезан заголовок"))?
            .try_into()
            .unwrap(),
    ))
}
fn chunks(bytes: &[u8], range: Range<usize>) -> Result<Vec<Chunk>> {
    let mut result = Vec::new();
    let mut at = range.start;
    while at < range.end {
        if range.end - at < 8 || range.end > bytes.len() {
            return Err(invalid("обрезан RIFF-чанк"));
        }
        let size = u32_at(bytes, at + 4)? as usize;
        let end = (at + 8)
            .checked_add(size)
            .filter(|&end| end <= range.end)
            .ok_or_else(|| invalid("чанк выходит за границы LIST/RIFF"))?;
        let next = end
            .checked_add(size & 1)
            .filter(|&end| end <= range.end)
            .ok_or_else(|| invalid("нет байта выравнивания RIFF"))?;
        result.push(Chunk {
            tag: bytes[at..at + 4].try_into().unwrap(),
            data: at + 8..end,
        });
        if result.len() > 1_000_000 {
            return Err(invalid("слишком много чанков"));
        }
        at = next;
    }
    Ok(result)
}
fn list(bytes: &[u8], c: &Chunk, kind: &[u8; 4]) -> bool {
    c.tag == *b"LIST"
        && bytes.get(c.data.start..c.data.start + 4) == Some(kind.as_slice())
        && c.data.len() >= 4
}
impl AviMovie {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > 512 * 1024 * 1024 {
            return Err(invalid("файл больше 512 МиБ"));
        }
        let mut headers = None;
        let mut movies = Vec::new();
        for (part, riff) in chunks(&bytes, 0..bytes.len())?.into_iter().enumerate() {
            let kind = if part == 0 { b"AVI " } else { b"AVIX" };
            if riff.tag != *b"RIFF"
                || riff.data.len() < 4
                || &bytes[riff.data.start..riff.data.start + 4] != kind
            {
                return Err(invalid("ожидался RIFF AVI (или продолжение AVIX)"));
            }
            for c in chunks(&bytes, riff.data.start + 4..riff.data.end)? {
                if list(&bytes, &c, b"hdrl") {
                    if headers.is_some() {
                        return Err(invalid("повтор hdrl"));
                    }
                    headers = Some(c.data.start + 4..c.data.end);
                } else if list(&bytes, &c, b"movi") {
                    movies.push(c.data.start + 4..c.data.end);
                }
            }
        }
        let mut streams = Vec::new();
        let mut main = false;
        for c in chunks(&bytes, headers.ok_or_else(|| invalid("нет hdrl"))?)? {
            if c.tag == *b"avih" {
                if main || c.data.len() < 56 {
                    return Err(invalid("неверный avih"));
                }
                main = true;
                if u32_at(&bytes, c.data.start + 12)? & 0x20 != 0 {
                    return Err(invalid("MUSTUSEINDEX пока не поддерживается"));
                }
            } else if list(&bytes, &c, b"strl") {
                let mut header = None;
                let mut format = None;
                for s in chunks(&bytes, c.data.start + 4..c.data.end)? {
                    let slot = if s.tag == *b"strh" {
                        &mut header
                    } else if s.tag == *b"strf" {
                        &mut format
                    } else {
                        continue;
                    };
                    if slot.replace(s.data).is_some() {
                        return Err(invalid("повтор заголовка потока"));
                    }
                }
                let header = header
                    .filter(|h| h.len() >= 56)
                    .ok_or_else(|| invalid("нет strh"))?;
                let format = format.ok_or_else(|| invalid("нет strf"))?;
                if u32_at(&bytes, header.start + 28)? != 0
                    || u32_at(&bytes, header.start + 8)? & 1 != 0
                {
                    return Err(invalid(
                        "отложенный или отключённый поток пока не поддерживается",
                    ));
                }
                streams.push(Stream {
                    kind: bytes[header.start..header.start + 4].try_into().unwrap(),
                    header,
                    format,
                });
                if streams.len() > 16 {
                    return Err(invalid("слишком много потоков"));
                }
            }
        }
        if !main {
            return Err(invalid("нет avih"));
        }
        let mut video_stream = None;
        let mut audio_stream = None;
        let (mut width, mut height, mut scale, mut rate) = (0, 0, 0, 0);
        let mut audio = None;
        for (index, s) in streams.iter().enumerate() {
            let f = &bytes[s.format.clone()];
            match &s.kind {
                b"vids" => {
                    if video_stream.replace(index).is_some() || f.len() < 40 {
                        return Err(invalid("ожидался один видеопоток BITMAPINFO"));
                    }
                    if u32_at(f, 0)? < 40
                        || u32_at(f, 0)? as usize > f.len()
                        || &f[16..20] != b"MJPG"
                    {
                        return Err(invalid("видеокодек не поддерживается; нужен MJPG"));
                    }
                    width = u32_at(f, 4)?;
                    height = u32_at(f, 8)?;
                    if width == 0
                        || height == 0
                        || width > 8192
                        || height > 8192
                        || u64::from(width) * u64::from(height) > 16_777_216
                    {
                        return Err(invalid("неверные или слишком большие размеры видео"));
                    }
                    scale = u32_at(&bytes, s.header.start + 20)?;
                    rate = u32_at(&bytes, s.header.start + 24)?;
                    if scale == 0 || rate == 0 || f64::from(rate) / f64::from(scale) > 240.0 {
                        return Err(invalid("неверная частота кадров"));
                    }
                }
                b"auds" => {
                    if audio_stream.replace(index).is_some() || f.len() < 16 {
                        return Err(invalid("ожидался один аудиопоток WAVEFORMAT"));
                    }
                    let channels = u16_at(f, 2)?;
                    let sample_rate = u32_at(f, 4)?;
                    if u16_at(f, 0)? != 1
                        || u16_at(f, 14)? != 16
                        || !(1..=2).contains(&channels)
                        || !(8000..=192000).contains(&sample_rate)
                        || u16_at(f, 12)? != channels * 2
                        || u32_at(f, 8)? != sample_rate * u32::from(channels) * 2
                    {
                        return Err(invalid(
                            "аудиокодек не поддерживается; нужен PCM16LE mono/stereo",
                        ));
                    }
                    audio = Some(AviAudio {
                        channels,
                        sample_rate,
                        samples: 0,
                    });
                }
                _ => return Err(invalid("тип потока пока не поддерживается")),
            }
        }
        let video_stream = video_stream.ok_or_else(|| invalid("нет видео"))?;
        let (mut video, mut audio_chunks) = (Vec::new(), Vec::new());
        for range in movies {
            collect_packets(
                &bytes,
                range,
                video_stream,
                audio_stream,
                &mut video,
                &mut audio_chunks,
                0,
            )?;
        }
        if video.is_empty() || video[0].is_empty() {
            return Err(invalid("нет первого видеокадра"));
        }
        // Нулевой видеочанк означает повтор предшествующего независимого кадра.
        for index in 1..video.len() {
            if video[index].is_empty() {
                video[index] = video[index - 1].clone();
            }
        }
        let duration = video.len() as f64 * f64::from(scale) / f64::from(rate);
        if duration > 86400.0 {
            return Err(invalid("видео длиннее суток"));
        }
        if let Some(info) = &mut audio {
            for c in &audio_chunks {
                if c.len() % (usize::from(info.channels) * 2) != 0 {
                    return Err(invalid("обрезан аудиокадр PCM"));
                }
                info.samples += c.len() / 2;
            }
            if info.samples == 0 {
                return Err(invalid("пустой аудиопоток"));
            }
        }
        Ok(Self {
            bytes: bytes.into(),
            video,
            audio_chunks,
            width,
            height,
            scale,
            rate,
            audio,
        })
    }
    pub fn frame_count(&self) -> usize {
        self.video.len()
    }
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(
            self.frame_count() as f64 * f64::from(self.scale) / f64::from(self.rate),
        )
    }
    pub fn frame_at(&self, time: Duration) -> usize {
        ((time.as_secs_f64() * f64::from(self.rate) / f64::from(self.scale)) as usize)
            .min(self.frame_count() - 1)
    }
    pub fn frame(&self, index: usize) -> Option<&[u8]> {
        self.video.get(index).map(|r| &self.bytes[r.clone()])
    }
    pub fn pcm(self: &Arc<Self>) -> Option<PcmSamples> {
        self.audio.map(|a| PcmSamples {
            movie: self.clone(),
            chunk: 0,
            offset: 0,
            remaining: a.samples,
        })
    }
}
#[allow(clippy::too_many_arguments)]
fn collect_packets(
    bytes: &[u8],
    range: Range<usize>,
    video_stream: usize,
    audio_stream: Option<usize>,
    video: &mut Vec<Range<usize>>,
    audio: &mut Vec<Range<usize>>,
    depth: usize,
) -> Result<()> {
    if depth > 8 {
        return Err(invalid("слишком глубокий LIST rec"));
    }
    for c in chunks(bytes, range)? {
        if list(bytes, &c, b"rec ") {
            collect_packets(
                bytes,
                c.data.start + 4..c.data.end,
                video_stream,
                audio_stream,
                video,
                audio,
                depth + 1,
            )?;
        } else if c.tag[..2].iter().all(u8::is_ascii_digit) {
            let stream = usize::from(c.tag[0] - b'0') * 10 + usize::from(c.tag[1] - b'0');
            if stream == video_stream && matches!(&c.tag[2..], b"dc" | b"db") {
                if c.data.len() > 32 * 1024 * 1024 {
                    return Err(invalid("кадр больше 32 МиБ"));
                }
                video.push(c.data);
            } else if Some(stream) == audio_stream && &c.tag[2..] == b"wb" {
                audio.push(c.data);
            } else {
                return Err(invalid("неизвестный пакет потока"));
            }
        } else if c.tag != *b"JUNK" && c.tag != *b"idx1" && &c.tag[..2] != b"ix" {
            return Err(invalid("неизвестный чанк movi"));
        }
        if video.len() + audio.len() > 1_000_000 {
            return Err(invalid("слишком много пакетов"));
        }
    }
    Ok(())
}
/// PCM читается прямо из AVI; при stereo сохраняется порядок L, R.
pub struct PcmSamples {
    movie: Arc<AviMovie>,
    chunk: usize,
    offset: usize,
    remaining: usize,
}
impl PcmSamples {
    /// Перемотка к границе аудиокадра. Без выделения нового буфера.
    pub fn seek(&mut self, time: Duration) {
        let info = self.movie.audio.unwrap();
        let frame = (time.as_secs_f64() * f64::from(info.sample_rate)) as usize;
        let samples = frame
            .saturating_mul(usize::from(info.channels))
            .min(info.samples);
        self.remaining = info.samples - samples;
        self.chunk = 0;
        self.offset = samples * 2;
        while let Some(c) = self.movie.audio_chunks.get(self.chunk) {
            if self.offset < c.len() {
                break;
            }
            self.offset -= c.len();
            self.chunk += 1;
        }
    }
}
impl Iterator for PcmSamples {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        while let Some(c) = self.movie.audio_chunks.get(self.chunk) {
            if self.offset == c.len() {
                self.chunk += 1;
                self.offset = 0;
                continue;
            }
            let at = c.start + self.offset;
            self.offset += 2;
            self.remaining -= 1;
            return Some(
                f32::from(i16::from_le_bytes([
                    self.movie.bytes[at],
                    self.movie.bytes[at + 1],
                ])) / 32768.0,
            );
        }
        None
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl ExactSizeIterator for PcmSamples {}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut b = tag.to_vec();
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        if data.len() % 2 == 1 {
            b.push(0);
        }
        b
    }
    fn list_chunk(tag: &[u8; 4], kind: &[u8; 4], contents: Vec<u8>) -> Vec<u8> {
        let mut b = kind.to_vec();
        b.extend(contents);
        chunk(tag, &b)
    }
    fn fixture(channels: u16, codec: &[u8; 4], extra: Vec<u8>) -> Vec<u8> {
        let mut h = vec![0; 56];
        h[..4].copy_from_slice(b"vids");
        h[20..24].copy_from_slice(&1u32.to_le_bytes());
        h[24..28].copy_from_slice(&25u32.to_le_bytes());
        let mut f = vec![0; 40];
        f[..4].copy_from_slice(&40u32.to_le_bytes());
        f[4..8].copy_from_slice(&640u32.to_le_bytes());
        f[8..12].copy_from_slice(&480u32.to_le_bytes());
        f[16..20].copy_from_slice(codec);
        let mut hdrl = chunk(b"avih", &[0; 56]);
        hdrl.extend(list_chunk(
            b"LIST",
            b"strl",
            [chunk(b"strh", &h), chunk(b"strf", &f)].concat(),
        ));
        if channels > 0 {
            h[..4].copy_from_slice(b"auds");
            let mut a = Vec::new();
            a.extend(1u16.to_le_bytes());
            a.extend(channels.to_le_bytes());
            a.extend(8000u32.to_le_bytes());
            a.extend((8000 * u32::from(channels) * 2).to_le_bytes());
            a.extend((channels * 2).to_le_bytes());
            a.extend(16u16.to_le_bytes());
            hdrl.extend(list_chunk(
                b"LIST",
                b"strl",
                [chunk(b"strh", &h), chunk(b"strf", &a)].concat(),
            ));
        }
        let mut packets = chunk(b"JUNK", &[9]);
        packets.extend(chunk(b"00dc", &[1, 2, 3]));
        packets.extend(chunk(b"00dc", &[]));
        if channels > 0 {
            packets.extend(chunk(
                b"01wb",
                &[-32768i16, 32767, 0, 16384]
                    .into_iter()
                    .flat_map(i16::to_le_bytes)
                    .collect::<Vec<_>>(),
            ));
        }
        packets.extend(extra);
        list_chunk(
            b"RIFF",
            b"AVI ",
            [
                list_chunk(b"LIST", b"hdrl", hdrl),
                list_chunk(b"LIST", b"movi", list_chunk(b"LIST", b"rec ", packets)),
            ]
            .concat(),
        )
    }
    #[test]
    fn nested_packets_padding_repeat_and_rational_clock() {
        let m = AviMovie::from_bytes(fixture(0, b"MJPG", vec![])).unwrap();
        assert_eq!(m.frame_count(), 2);
        assert_eq!(m.frame(0), m.frame(1));
        assert_eq!(m.frame(2), None);
        assert_eq!(m.duration(), Duration::from_millis(80));
        assert_eq!(m.frame_at(Duration::from_millis(39)), 0);
        assert_eq!(m.frame_at(Duration::from_millis(40)), 1);
        assert_eq!(m.frame_at(Duration::MAX), 1);
    }
    #[test]
    fn pcm_sign_stereo_chunks_seek_and_end() {
        let m = Arc::new(
            AviMovie::from_bytes(fixture(2, b"MJPG", chunk(b"01wb", &[0, 64, 0, 192]))).unwrap(),
        );
        let mut pcm = m.pcm().unwrap();
        assert_eq!(pcm.len(), 6);
        assert_eq!(pcm.next(), Some(-1.0));
        assert_eq!(pcm.next(), Some(32767.0 / 32768.0));
        pcm.seek(Duration::from_secs_f64(1.0 / 8000.0));
        assert_eq!(pcm.collect::<Vec<_>>(), [0.0, 0.5, 0.5, -0.5]);
        let mut pcm = m.pcm().unwrap();
        pcm.seek(Duration::MAX);
        assert_eq!(pcm.next(), None);
        assert_eq!(pcm.len(), 0);
    }
    #[test]
    fn avix_continuation() {
        let mut b = fixture(0, b"MJPG", vec![]);
        b.extend(list_chunk(
            b"RIFF",
            b"AVIX",
            list_chunk(b"LIST", b"movi", chunk(b"00dc", &[7, 8])),
        ));
        let m = AviMovie::from_bytes(b).unwrap();
        assert_eq!(m.frame_count(), 3);
        assert_eq!(m.frame(2), Some([7, 8].as_slice()));
    }
    #[test]
    fn every_truncated_prefix_is_rejected_without_panic() {
        let b = fixture(1, b"MJPG", vec![]);
        for i in 0..b.len() {
            assert!(AviMovie::from_bytes(b[..i].to_vec()).is_err(), "prefix {i}");
        }
    }
    #[test]
    fn oversized_chunk_and_missing_alignment_rejected() {
        let mut b = fixture(0, b"MJPG", vec![]);
        b[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(AviMovie::from_bytes(b).is_err());
        let b = chunk(b"JUNK", &[1]);
        assert!(chunks(&b, 0..b.len() - 1).is_err());
    }
    #[test]
    fn unsupported_codec_and_broken_pcm_are_clear_errors() {
        assert!(
            AviMovie::from_bytes(fixture(0, b"IV50", vec![]))
                .unwrap_err()
                .to_string()
                .contains("MJPG")
        );
        assert!(
            AviMovie::from_bytes(fixture(2, b"MJPG", chunk(b"01wb", &[0, 0])))
                .unwrap_err()
                .to_string()
                .contains("PCM")
        );
    }
    #[test]
    fn dimensions_rate_and_stream_start_are_checked() {
        for (tag, offset, value) in [(b"strf", 4, u32::MAX), (b"strh", 20, 0), (b"strh", 28, 1)] {
            let mut b = fixture(0, b"MJPG", vec![]);
            let at = b.windows(4).position(|w| w == tag).unwrap() + 8 + offset;
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(AviMovie::from_bytes(b).is_err());
        }
    }
    #[test]
    fn deep_record_lists_and_unknown_packets_rejected() {
        let mut rec = chunk(b"00dc", &[1]);
        for _ in 0..10 {
            rec = list_chunk(b"LIST", b"rec ", rec);
        }
        assert!(AviMovie::from_bytes(fixture(0, b"MJPG", rec)).is_err());
        assert!(AviMovie::from_bytes(fixture(0, b"MJPG", chunk(b"01dc", &[1]))).is_err());
    }
}
