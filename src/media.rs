//! Read-only ISO9660/Joliet and single-volume InstallShield 5 media.
//! InstallShield layout reference: twogood/unshield (MIT); see third-party notices.
use anyhow::{Context, Result, ensure};
use flate2::read::DeflateDecoder;
use std::{
    collections::{BTreeSet, VecDeque},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const SECTOR: u64 = 2048;
const MAX_FILES: usize = 100_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub path: String,
    offset: u64,
    stored: u64,
    pub size: u64,
    compressed: bool,
    obfuscated: bool,
}
pub(crate) struct Media {
    file: Source,
    pub entries: Vec<Entry>,
}
/// Logical 2048-byte data sectors over flat ISO or raw MDF sectors.
struct Source {
    file: File,
    start: u64,
    stride: u64,
    payload: u64,
    length: u64,
    cursor: u64,
}
impl Source {
    fn plain(file: File) -> Result<Self> {
        let length = file.metadata()?.len();
        Ok(Self {
            file,
            start: 0,
            stride: 2048,
            payload: 0,
            length,
            cursor: 0,
        })
    }
    fn len(&self) -> u64 {
        self.length
    }
    fn probe(
        path: &Path,
        start: u64,
        stride: u64,
        payload: u64,
        sectors: Option<u64>,
    ) -> Result<Self> {
        ensure!(
            stride >= 2048 && payload + 2048 <= stride,
            "Неверный размер сектора MDF"
        );
        let file = File::open(path)?;
        let physical = file.metadata()?.len();
        ensure!(start <= physical, "Дорожка за пределами MDF");
        let available = (physical - start) / stride;
        let blocks = sectors.unwrap_or(available).min(available);
        let mut source = Self {
            file,
            start,
            stride,
            payload,
            length: blocks * 2048,
            cursor: 0,
        };
        let descriptor = extent(&mut source, 16 * 2048, 2048, 2048)?;
        ensure!(
            &descriptor[1..6] == b"CD001" && descriptor[6] == 1,
            "На дорожке нет ISO9660"
        );
        Ok(source)
    }
    fn disc(path: &Path) -> Result<Self> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let descriptor = if ext == "mds" {
            Some(path.to_owned())
        } else if ext == "mdf" {
            sibling(path, "mds")
        } else {
            None
        };
        let image = if ext == "mds" {
            sibling(path, "mdf")
        } else {
            Some(path.to_owned())
        };
        if let Some(descriptor) = descriptor {
            ensure!(
                (88..=4 * 1024 * 1024).contains(&std::fs::metadata(&descriptor)?.len()),
                "Некорректный размер MDS"
            );
            let b = std::fs::read(&descriptor)?;
            ensure!(
                b.len() >= 88
                    && b.len() <= 4 * 1024 * 1024
                    && &b[..16] == b"MEDIA DESCRIPTOR"
                    && b[16] == 1,
                "Не поддерживается эта версия MDS"
            );
            let sessions = u16le(&b, 20)? as usize;
            ensure!(sessions > 0 && sessions <= 99, "Неверное число сессий MDS");
            let offset = u32le(&b, 80)? as usize;
            for i in 0..sessions {
                let session = offset.checked_add(i * 24).context("Неверная сессия MDS")?;
                let block = b
                    .get(session..session + 24)
                    .context("Обрезанная сессия MDS")?;
                let tracks = u32le(block, 20)? as usize;
                for j in 0..block[10] as usize {
                    let t = tracks.checked_add(j * 80).context("Неверная дорожка MDS")?;
                    let t = b.get(t..t + 80).context("Обрезанная дорожка MDS")?;
                    if !(1..=99).contains(&t[4]) || t[0] == 0xa9 {
                        continue;
                    }
                    ensure!(
                        u32le(t, 48)? <= 1,
                        "Разделённые MDF/MD0 пока не поддерживаются"
                    );
                    let stride = u16le(t, 16)? as u64;
                    let start = u64::from_le_bytes(t[40..48].try_into()?);
                    let extra = u32le(t, 12)? as usize;
                    let sectors = if extra > 0 {
                        Some(if u16le(&b, 18)? & 0x10 != 0 {
                            extra as u64
                        } else {
                            u32le(&b, extra + 4)? as u64
                        })
                    } else {
                        None
                    };
                    let track_image = if let Some(image) = &image {
                        image.clone()
                    } else {
                        mds_filename(&descriptor, &b, t)?
                    };
                    let options: &[u64] = match stride {
                        2048 => &[0],
                        2336 => &[8, 0],
                        2352 | 2448 => &[16, 24],
                        _ => &[],
                    };
                    for &payload in options {
                        if let Ok(source) =
                            Self::probe(&track_image, start, stride, payload, sectors)
                        {
                            return Ok(source);
                        }
                    }
                }
            }
            anyhow::bail!("В MDS не найдена поддерживаемая дорожка с файлами игры");
        }
        let image = image.context("Файл MDF не найден")?;
        // A lone MDF can still be read when its data track starts at the beginning.
        for (stride, payload) in [
            (2048, 0),
            (2352, 16),
            (2352, 24),
            (2448, 16),
            (2448, 24),
            (2336, 8),
        ] {
            if let Ok(source) = Self::probe(&image, 0, stride, payload, None) {
                return Ok(source);
            }
        }
        anyhow::bail!("Не удалось прочитать образ. Для MDF выбери его парный MDS");
    }
}
fn mds_filename(descriptor: &Path, bytes: &[u8], track: &[u8]) -> Result<PathBuf> {
    let footer = u32le(track, 52)? as usize;
    ensure!(footer > 0, "В MDS нет имени MDF");
    let offset = u32le(bytes, footer)? as usize;
    let filename = if u32le(bytes, footer + 4)? != 0 {
        let raw = bytes.get(offset..).context("Обрезанное имя MDF")?;
        let values = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| u16::from_le_bytes(*v))
            .take_while(|x| *x != 0)
            .collect::<Vec<_>>();
        ensure!(values.len() <= 512, "Слишком длинное имя MDF");
        String::from_utf16(&values)?
    } else {
        c_string(bytes, offset)?
    };
    let name = filename
        .rsplit(['\\', '/'])
        .next()
        .context("Пустое имя MDF")?;
    ensure!(
        !name.is_empty() && name != "." && name != "..",
        "Неверное имя MDF"
    );
    let expected = if name == "*.mdf" || name == "*.MDF" {
        format!(
            "{}.mdf",
            descriptor
                .file_stem()
                .context("Нет имени MDS")?
                .to_string_lossy()
        )
    } else {
        name.to_owned()
    };
    let parent = descriptor.parent().context("Нет папки MDS")?;
    std::fs::read_dir(parent)?
        .filter_map(|e| e.ok())
        .find_map(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.to_lowercase() == expected.to_lowercase())
                .then(|| e.path())
        })
        .context("Рядом с MDS не найден соответствующий MDF")
}
fn sibling(path: &Path, extension: &str) -> Option<PathBuf> {
    let expected = format!("{}.{}", path.file_stem()?.to_str()?, extension);
    std::fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(|e| e.ok())
        .find_map(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(&expected))
                .then(|| e.path())
        })
}
impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cursor >= self.length || buffer.is_empty() {
            return Ok(0);
        }
        let mut done = 0;
        let wanted = (self.length - self.cursor).min(buffer.len() as u64) as usize;
        if self.stride == 2048 && self.payload == 0 {
            self.file.seek(SeekFrom::Start(self.start + self.cursor))?;
            let n = self.file.read(&mut buffer[..wanted])?;
            self.cursor += n as u64;
            return Ok(n);
        }
        while done < wanted {
            let sector = self.cursor / 2048;
            let inside = self.cursor % 2048;
            let n = (2048 - inside).min((wanted - done) as u64) as usize;
            self.file.seek(SeekFrom::Start(
                self.start + sector * self.stride + self.payload + inside,
            ))?;
            self.file.read_exact(&mut buffer[done..done + n])?;
            self.cursor += n as u64;
            done += n;
        }
        Ok(done)
    }
}
impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let next = match position {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.cursor as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if next < 0 || next > u64::MAX as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Неверное смещение",
            ));
        }
        self.cursor = next as u64;
        Ok(self.cursor)
    }
}

fn u16le(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2)
            .context("Обрезанный заголовок носителя")?
            .try_into()?,
    ))
}
fn u32le(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("Обрезанный заголовок носителя")?
            .try_into()?,
    ))
}
fn c_string(b: &[u8], o: usize) -> Result<String> {
    let s = b.get(o..).context("Имя за пределами заголовка")?;
    let n = s
        .iter()
        .position(|x| *x == 0)
        .context("Незавершённое имя")?;
    ensure!(n <= 240, "Слишком длинное имя");
    String::from_utf8(s[..n].to_vec()).context("Неподдерживаемая кодировка имён установщика")
}
fn extent(file: &mut Source, offset: u64, size: u64, maximum: u64) -> Result<Vec<u8>> {
    ensure!(
        size <= maximum && offset.checked_add(size).is_some_and(|n| n <= file.len()),
        "Область файла выходит за пределы носителя"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; usize::try_from(size)?];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn validate(entries: &[Entry], length: u64) -> Result<()> {
    ensure!(
        !entries.is_empty() && entries.len() <= MAX_FILES,
        "Некорректное число файлов на носителе"
    );
    let mut paths = BTreeSet::new();
    let mut total = 0u64;
    for e in entries {
        crate::manifest::validate_path(&e.path)?;
        ensure!(
            paths.insert(e.path.to_lowercase()),
            "Повторяющийся путь: {}",
            e.path
        );
        ensure!(
            e.offset.checked_add(e.stored).is_some_and(|n| n <= length),
            "Файл выходит за пределы носителя: {}",
            e.path
        );
        total = total
            .checked_add(e.size)
            .context("Слишком большой носитель")?;
        ensure!(total <= MAX_BYTES, "Игровые данные больше 32 ГБ");
    }
    Ok(())
}
impl Media {
    pub fn iso(path: &Path) -> Result<Self> {
        let mut file = Source::disc(path)?;
        let length = file.len();
        let mut root = None;
        let mut joliet = false;
        for sector in 16..80 {
            let b = extent(&mut file, sector * SECTOR, SECTOR, SECTOR)?;
            ensure!(
                &b[1..6] == b"CD001" && b[6] == 1,
                "Нужен образ ISO9660/Joliet (.iso)"
            );
            if b[0] == 255 {
                break;
            }
            let unicode = b[0] == 2 && matches!(&b[88..91], b"%/@" | b"%/C" | b"%/E");
            if (b[0] == 1 && root.is_none()) || unicode {
                ensure!(
                    u16le(&b, 128)? == 2048,
                    "Неподдерживаемый размер сектора ISO"
                );
                root = Some((u32le(&b, 158)? as u64 * SECTOR, u32le(&b, 166)? as u64));
                joliet = unicode;
            }
        }
        let (offset, size) = root.context("В образе не найден каталог ISO9660")?;
        let mut queue = VecDeque::from([(String::new(), offset, size, 0)]);
        let mut visited = BTreeSet::new();
        let mut entries = Vec::new();
        while let Some((parent, offset, size, depth)) = queue.pop_front() {
            ensure!(
                depth <= 48 && visited.insert((offset, size)),
                "Циклический или слишком глубокий каталог ISO"
            );
            let bytes = extent(&mut file, offset, size, 8 * 1024 * 1024)?;
            let mut p = 0usize;
            while p < bytes.len() {
                let n = bytes[p] as usize;
                if n == 0 {
                    p = (p / 2048 + 1) * 2048;
                    continue;
                }
                ensure!(
                    n >= 34 && p + n <= bytes.len(),
                    "Обрезанная запись каталога ISO"
                );
                let r = &bytes[p..p + n];
                p += n;
                let name_len = r[32] as usize;
                let name = r.get(33..33 + name_len).context("Обрезанное имя ISO")?;
                if name == [0] || name == [1] {
                    continue;
                }
                let name = if joliet {
                    ensure!(name.len() % 2 == 0, "Повреждённое имя Joliet");
                    String::from_utf16(
                        &name
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|x| u16::from_be_bytes([x[0], x[1]]))
                            .collect::<Vec<_>>(),
                    )?
                } else {
                    std::str::from_utf8(name)?.to_owned()
                };
                let name = name.split(';').next().unwrap_or("").trim_end_matches('.');
                crate::manifest::validate_path(name)?;
                ensure!(!name.contains('/'), "Некорректное имя ISO");
                let path = if parent.is_empty() {
                    name.into()
                } else {
                    format!("{parent}/{name}")
                };
                crate::manifest::validate_path(&path)?;
                ensure!(
                    r[25] & 0x80 == 0 && r[26] == 0 && r[27] == 0,
                    "Разделённые и чередующиеся файлы ISO пока не поддерживаются"
                );
                let start = u32le(r, 2)? as u64 * SECTOR;
                let size = u32le(r, 10)? as u64;
                if r[25] & 2 != 0 {
                    queue.push_back((path, start, size, depth + 1));
                    ensure!(
                        queue.len() + visited.len() <= MAX_FILES,
                        "Слишком много каталогов ISO"
                    );
                } else {
                    entries.push(Entry {
                        path,
                        offset: start,
                        stored: size,
                        size,
                        compressed: false,
                        obfuscated: false,
                    });
                    ensure!(entries.len() <= MAX_FILES, "Слишком много файлов ISO");
                }
            }
        }
        validate(&entries, length)?;
        Ok(Self { file, entries })
    }
    pub fn installshield5(header: &Path, cabinet: &Path) -> Result<Self> {
        let mut h = Source::plain(File::open(header)?)?;
        let size = h.len();
        let bytes = extent(&mut h, 0, size, 16 * 1024 * 1024)?;
        ensure!(
            u32le(&bytes, 0)? == 0x28635349 && (u32le(&bytes, 4)? >> 12) & 15 == 5,
            "Поддерживается оригинальный InstallShield 5"
        );
        let base = u32le(&bytes, 12)? as usize;
        let descriptor_size = u32le(&bytes, 16)? as usize;
        ensure!(
            base.checked_add(descriptor_size)
                .is_some_and(|n| n <= bytes.len())
                && descriptor_size >= 48,
            "Обрезанный описатель InstallShield"
        );
        let table = base
            .checked_add(u32le(&bytes, base + 12)? as usize)
            .context("Неверный адрес таблицы")?;
        let dirs = u32le(&bytes, base + 28)? as usize;
        let files = u32le(&bytes, base + 40)? as usize;
        ensure!(
            dirs <= MAX_FILES && files <= MAX_FILES && dirs + files > 0,
            "Неверное число файлов InstallShield"
        );
        ensure!(
            table
                .checked_add((dirs + files) * 4)
                .is_some_and(|n| n <= bytes.len()),
            "Обрезанная таблица InstallShield"
        );
        let mut directories = Vec::new();
        for i in 0..dirs {
            let o = table
                .checked_add(u32le(&bytes, table + i * 4)? as usize)
                .context("Неверный каталог")?;
            directories.push(c_string(&bytes, o)?.replace('\\', "/"));
        }
        let mut entries = Vec::new();
        for i in 0..files {
            let o = table
                .checked_add(u32le(&bytes, table + (dirs + i) * 4)? as usize)
                .context("Неверная запись файла")?;
            let flags = u16le(&bytes, o + 8)?;
            let offset = u32le(&bytes, o + 38)? as u64;
            let name = u32le(&bytes, o)? as usize;
            if flags & 8 != 0 || name == 0 || offset == 0 {
                continue;
            }
            ensure!(
                flags & !14 == 0,
                "Многотомный InstallShield пока не поддерживается"
            );
            let directory = directories
                .get(u32le(&bytes, o + 4)? as usize)
                .context("Некорректный каталог InstallShield")?;
            let name = c_string(&bytes, table.checked_add(name).context("Неверное имя")?)?;
            ensure!(
                !name.contains(['\\', '/']),
                "Некорректное имя в InstallShield"
            );
            let path = if directory.is_empty() {
                name
            } else {
                format!("{directory}/{name}")
            };
            let size = u32le(&bytes, o + 10)? as u64;
            let compressed = flags & 4 != 0;
            entries.push(Entry {
                path,
                offset,
                stored: if compressed {
                    u32le(&bytes, o + 14)? as u64
                } else {
                    size
                },
                size,
                compressed,
                obfuscated: flags & 2 != 0,
            });
        }
        let mut file = Source::plain(File::open(cabinet)?)?;
        let header = extent(&mut file, 0, 20, 20)?;
        ensure!(
            u32le(&header, 0)? == 0x28635349,
            "Повреждённый файл data1.cab"
        );
        validate(&entries, file.len())?;
        Ok(Self { file, entries })
    }
    pub fn total(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
    pub fn extract(
        &mut self,
        destination: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(&str, u64, u64),
    ) -> Result<()> {
        let total = self.total();
        let mut done = 0u64;
        for entry in &self.entries {
            ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
            let path = destination.join(&entry.path);
            fs::create_dir_all(path.parent().context("Нет родительского каталога")?)?;
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            self.file.seek(SeekFrom::Start(entry.offset))?;
            let mut left = entry.stored;
            let mut written = 0u64;
            let mut seed = 0u64;
            let mut read = |bytes: &mut [u8]| -> Result<()> {
                ensure!(bytes.len() as u64 <= left, "Обрезанные данные установщика");
                self.file.read_exact(bytes)?;
                left -= bytes.len() as u64;
                if entry.obfuscated {
                    for b in bytes {
                        *b = (*b ^ 0xd5)
                            .rotate_right(2)
                            .wrapping_sub((seed % 0x47) as u8);
                        seed += 1;
                    }
                }
                Ok(())
            };
            if entry.compressed {
                let mut consumed = 0;
                while consumed < entry.stored {
                    ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
                    let mut header = [0; 2];
                    read(&mut header)?;
                    let n = u16::from_le_bytes(header) as usize;
                    ensure!(n > 0, "Пустой блок InstallShield");
                    let mut encoded = vec![0; n + 1];
                    read(&mut encoded[..n])?;
                    let mut chunk = Vec::new();
                    DeflateDecoder::new(&encoded[..])
                        .take(65537)
                        .read_to_end(&mut chunk)?;
                    ensure!(
                        !chunk.is_empty()
                            && chunk.len() <= 65536
                            && written + chunk.len() as u64 <= entry.size,
                        "Некорректный распакованный блок"
                    );
                    output.write_all(&chunk)?;
                    written += chunk.len() as u64;
                    done += chunk.len() as u64;
                    consumed += n as u64 + 2;
                    progress(&entry.path, done, total);
                }
            } else {
                let mut copied = 0;
                let mut buffer = vec![0; 256 * 1024];
                while copied < entry.stored {
                    ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
                    let n = (entry.stored - copied).min(buffer.len() as u64) as usize;
                    read(&mut buffer[..n])?;
                    output.write_all(&buffer[..n])?;
                    copied += n as u64;
                    written += n as u64;
                    done += n as u64;
                    progress(&entry.path, done, total);
                }
            }
            ensure!(
                written == entry.size,
                "Размер распакованного файла не совпал: {}",
                entry.path
            );
            output.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    pub fn gut() -> Vec<u8> {
        let names = [
            "config/window_bases/main_menu.txt",
            "campaign/training/training.txt",
        ];
        let mut b = vec![0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x13];
        b.extend(2u32.to_le_bytes());
        b.extend([0; 32]);
        let mut offset = names.iter().map(|s| 20 + s.len() + 1).sum::<usize>();
        for name in names {
            for n in [name.len() as u32 + 1, 7, offset as u32, 0, 0] {
                b.extend(n.to_le_bytes());
            }
            b.extend(name.bytes().chain([0]).map(|x| x ^ 0xff));
            offset += 7;
        }
        b.extend(b"fixturefixture");
        b
    }
    fn record(name: &[u8], extent: u32, size: u32, dir: bool) -> Vec<u8> {
        let n = (34 + name.len()) & !1;
        let mut r = vec![0; n];
        r[0] = n as u8;
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[6..10].copy_from_slice(&extent.to_be_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[14..18].copy_from_slice(&size.to_be_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[28..30].copy_from_slice(&1u16.to_le_bytes());
        r[30..32].copy_from_slice(&1u16.to_be_bytes());
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r
    }
    pub fn iso(unicode: bool) -> Vec<u8> {
        let mut b = vec![0; 24 * 2048];
        let root = record(&[0], 20, 2048, true);
        for (sector, kind) in if unicode {
            vec![(16, 1), (17, 2), (18, 255)]
        } else {
            vec![(16, 1), (17, 255)]
        } {
            let d = &mut b[sector * 2048..(sector + 1) * 2048];
            d[0] = kind;
            d[1..6].copy_from_slice(b"CD001");
            d[6] = 1;
            d[80..84].copy_from_slice(&24u32.to_le_bytes());
            d[84..88].copy_from_slice(&24u32.to_be_bytes());
            d[128..130].copy_from_slice(&2048u16.to_le_bytes());
            d[130..132].copy_from_slice(&2048u16.to_be_bytes());
            d[156..190].copy_from_slice(&root);
            if kind == 2 {
                d[88..91].copy_from_slice(b"%/E");
            }
        }
        let payload = gut();
        let mut directory = Vec::new();
        directory.extend(&root);
        directory.extend(record(&[1], 20, 2048, true));
        let name = if unicode {
            "CORE.GUT;1"
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>()
        } else {
            b"CORE.GUT;1".to_vec()
        };
        directory.extend(record(&name, 21, payload.len() as u32, false));
        if unicode {
            let name = "Карта.bin;1"
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            directory.extend(record(&name, 22, 4, false));
            b[22 * 2048..22 * 2048 + 4].copy_from_slice(b"map!");
        }
        b[20 * 2048..20 * 2048 + directory.len()].copy_from_slice(&directory);
        b[21 * 2048..21 * 2048 + payload.len()].copy_from_slice(&payload);
        b
    }
    pub fn mdf(iso: &[u8], stride: usize, payload: usize, prefix: usize) -> (Vec<u8>, Vec<u8>) {
        let blocks = iso.len() / 2048;
        let mut raw = vec![0xee; prefix + blocks * stride];
        for i in 0..blocks {
            raw[prefix + i * stride + payload..prefix + i * stride + payload + 2048]
                .copy_from_slice(&iso[i * 2048..(i + 1) * 2048]);
        }
        let mut mds = vec![0; 240];
        mds[..16].copy_from_slice(b"MEDIA DESCRIPTOR");
        mds[16] = 1;
        mds[17] = 3;
        mds[20..22].copy_from_slice(&1u16.to_le_bytes());
        mds[80..84].copy_from_slice(&88u32.to_le_bytes());
        mds[88 + 10] = 1;
        mds[88 + 20..88 + 24].copy_from_slice(&112u32.to_le_bytes());
        let t = &mut mds[112..192];
        t[0] = 0xaa;
        t[4] = 1;
        t[12..16].copy_from_slice(&192u32.to_le_bytes());
        t[16..18].copy_from_slice(&(stride as u16).to_le_bytes());
        t[40..48].copy_from_slice(&(prefix as u64).to_le_bytes());
        t[48..52].copy_from_slice(&1u32.to_le_bytes());
        mds[196..200].copy_from_slice(&(blocks as u32).to_le_bytes());
        (raw, mds)
    }
    pub fn cab() -> (Vec<u8>, Vec<u8>) {
        use flate2::{Compression, write::DeflateEncoder};
        let data = gut();
        let mut compressed = DeflateEncoder::new(Vec::new(), Compression::default());
        compressed.write_all(&data).unwrap();
        let compressed = compressed.finish().unwrap();
        let mut stream = (compressed.len() as u16).to_le_bytes().to_vec();
        stream.extend(compressed);
        let mut hdr = vec![0; 1024];
        hdr[..4].copy_from_slice(&0x28635349u32.to_le_bytes());
        hdr[4..8].copy_from_slice(&0x01005201u32.to_le_bytes());
        hdr[12..16].copy_from_slice(&512u32.to_le_bytes());
        hdr[16..20].copy_from_slice(&256u32.to_le_bytes());
        for (o, v) in [
            (512 + 12, 256),
            (512 + 28, 1),
            (512 + 40, 1),
            (768, 8),
            (772, 32),
            (800, 13),
            (804, 0),
            (810, data.len() as u32),
            (814, stream.len() as u32),
            (838, 512),
        ] {
            hdr[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        hdr[776..781].copy_from_slice(b"Data\0");
        hdr[781..790].copy_from_slice(b"core.gut\0");
        hdr[808..810].copy_from_slice(&4u16.to_le_bytes());
        let mut cab = vec![0; 512];
        cab[..8].copy_from_slice(&hdr[..8]);
        cab.extend(stream);
        (hdr, cab)
    }
    #[test]
    fn iso_and_raw_mdf_sector_layouts_extract_identical_payloads() {
        for (stride, payload) in [
            (2048, 0),
            (2352, 16),
            (2352, 24),
            (2448, 16),
            (2448, 24),
            (2336, 8),
        ] {
            let d = tempfile::tempdir().unwrap();
            let iso = iso(true);
            let (raw, mds) = mdf(&iso, stride, payload, 8192);
            fs::write(d.path().join("Диск.MDF"), raw).unwrap();
            fs::write(d.path().join("Диск.mds"), mds).unwrap();
            let mut image = Media::iso(&d.path().join("Диск.mds")).unwrap();
            let out = d.path().join("out");
            image
                .extract(&out, &AtomicBool::new(false), |_, _, _| {})
                .unwrap();
            assert_eq!(fs::read(out.join("CORE.GUT")).unwrap(), gut());
            assert_eq!(fs::read(out.join("Карта.bin")).unwrap(), b"map!");
        }
    }
    #[test]
    fn truncated_mds_and_out_of_range_track_offsets_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let (raw, mut mds) = mdf(&iso(false), 2352, 16, 0);
        fs::write(d.path().join("disc.mdf"), raw).unwrap();
        mds[112 + 40..112 + 48].copy_from_slice(&u64::MAX.to_le_bytes());
        fs::write(d.path().join("disc.mds"), &mds).unwrap();
        assert!(Media::iso(&d.path().join("disc.mds")).is_err());
        fs::write(d.path().join("disc.mds"), &mds[..100]).unwrap();
        assert!(Media::iso(&d.path().join("disc.mds")).is_err());
    }
    #[test]
    fn installshield5_decompresses_and_rejects_out_of_bounds_entries() {
        let d = tempfile::tempdir().unwrap();
        let (mut hdr, cab) = cab();
        let h = d.path().join("data1.hdr");
        let c = d.path().join("data1.cab");
        fs::write(&h, &hdr).unwrap();
        fs::write(&c, cab).unwrap();
        let mut media = Media::installshield5(&h, &c).unwrap();
        media
            .extract(&d.path().join("out"), &AtomicBool::new(false), |_, _, _| {})
            .unwrap();
        assert_eq!(fs::read(d.path().join("out/Data/core.gut")).unwrap(), gut());
        hdr[838..842].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&h, &hdr).unwrap();
        assert!(Media::installshield5(&h, &c).is_err());
    }
    #[test]
    fn traversal_and_case_collisions_from_images_are_rejected() {
        let e = Entry {
            path: "../escape".into(),
            offset: 0,
            stored: 1,
            size: 1,
            compressed: false,
            obfuscated: false,
        };
        assert!(validate(std::slice::from_ref(&e), 1).is_err());
        let mut first = e;
        first.path = "Data/one".into();
        let mut second = first.clone();
        second.path = "data/ONE".into();
        assert!(validate(&[first, second], 1).is_err());
    }
}
