use crate::{Error, Result, normalize_path};
use encoding_rs::WINDOWS_1252;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAGIC: [u8; 8] = [0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x13];
const MAX_ENTRIES: u32 = 100_000;
const MAX_RESOURCE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GutEntry {
    pub name: String,
    pub size: u32,
    pub offset: u64,
    pub encoded: bool,
    /// Поле оригинального формата. Его алгоритм пока не установлен;
    /// это значение не используется как подтверждение целостности ресурса.
    pub stored_checksum: u32,
}

#[derive(Debug)]
pub struct GutArchive {
    path: PathBuf,
    pub name: String,
    pub entries: Vec<GutEntry>,
    lookup: BTreeMap<String, usize>,
}

impl GutArchive {
    /// Читаем только заголовок и таблицу. Все изображения сразу в RAM не загружаем.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        let mut prefix = vec![0; file_len.min(65_536) as usize];
        reader.read_exact(&mut prefix)?;
        let magic_at = prefix
            .windows(MAGIC.len())
            .position(|bytes| bytes == MAGIC)
            .ok_or_else(|| {
                Error::Invalid(format!("{}: сигнатура GUT не найдена", path.display()))
            })?;
        reader.seek(SeekFrom::Start((magic_at + MAGIC.len()) as u64))?;
        let count = read_u32(&mut reader)?;
        if count > MAX_ENTRIES || u64::from(count) > file_len / 20 {
            return Err(Error::Invalid("слишком много записей в таблице GUT".into()));
        }
        let mut raw_archive_name = [0; 32];
        reader.read_exact(&mut raw_archive_name)?;
        let name = decode_name(&raw_archive_name);
        // Смещения файлов отсчитываются от начала таблицы, а не начала ZIP/GUT.
        let table_base = reader.stream_position()?;
        let mut entries = Vec::with_capacity(count as usize);
        let mut lookup = BTreeMap::new();
        for index in 0..count {
            let name_len = read_u32(&mut reader)?;
            let size = read_u32(&mut reader)?;
            let relative_offset = read_u32(&mut reader)?;
            let encoded = read_u32(&mut reader)?;
            let stored_checksum = read_u32(&mut reader)?;
            if name_len == 0 || name_len > 4096 || encoded > 1 {
                return Err(Error::Invalid("неподдерживаемая запись таблицы GUT".into()));
            }
            let mut raw_name = vec![0; name_len as usize];
            reader.read_exact(&mut raw_name)?;
            for byte in &mut raw_name {
                *byte ^= 0xff;
            }
            let name = decode_name(&raw_name);
            let key = normalize_path(&name)?;
            let offset = table_base + u64::from(relative_offset);
            if offset + u64::from(size) > file_len {
                return Err(Error::Invalid(format!(
                    "{name}: данные выходят за конец архива"
                )));
            }
            if lookup.insert(key, index as usize).is_some() {
                return Err(Error::Invalid(format!("повторяющийся путь {name}")));
            }
            entries.push(GutEntry {
                name,
                size,
                offset,
                encoded: encoded != 0,
                stored_checksum,
            });
        }
        let table_end = reader.stream_position()?;
        if entries.iter().any(|entry| entry.offset < table_end) {
            return Err(Error::Invalid(
                "данные ресурса пересекаются с таблицей GUT".into(),
            ));
        }
        Ok(Self {
            path,
            name,
            entries,
            lookup,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let key = normalize_path(name)?;
        let index = self
            .lookup
            .get(&key)
            .ok_or_else(|| Error::NotFound(name.into()))?;
        self.read_entry(*index)
    }

    pub fn read_entry(&self, index: usize) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| Error::NotFound(format!("индекс {index}")))?;
        if u64::from(entry.size) > MAX_RESOURCE_BYTES {
            return Err(Error::Invalid(
                "один ресурс больше предела прототипа 512 МиБ".into(),
            ));
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut bytes = vec![0; entry.size as usize];
        file.read_exact(&mut bytes)?;
        if entry.encoded {
            for byte in &mut bytes {
                *byte ^= 0xff;
            }
        }
        Ok(bytes)
    }
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn decode_name(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    WINDOWS_1252.decode(&bytes[..end]).0.into_owned()
}
