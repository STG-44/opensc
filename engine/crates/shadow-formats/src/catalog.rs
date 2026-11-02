use crate::{Error, GutArchive, Result, normalize_path};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub enum AssetOrigin {
    Archive { archive: usize, entry: usize },
    Loose(PathBuf),
    Override(PathBuf),
}

/// Порядок замены: GUT < отдельные файлы оригинала < наши улучшения.
#[derive(Debug)]
pub struct AssetCatalog {
    pub archives: Vec<GutArchive>,
    assets: BTreeMap<String, AssetOrigin>,
}

impl AssetCatalog {
    pub fn open(game_dir: impl AsRef<Path>, overrides: Option<&Path>) -> Result<Self> {
        let game_dir = game_dir.as_ref();
        let data_dir = if game_dir.join("Data").is_dir() {
            game_dir.join("Data")
        } else if game_dir.join("data").is_dir() {
            game_dir.join("data")
        } else {
            game_dir.to_path_buf()
        };
        let mut paths = fs::read_dir(&data_dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        let mut archives = Vec::new();
        let mut assets = BTreeMap::new();
        for path in paths {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gut"))
                && path.is_file()
            {
                let archive = GutArchive::open(&path)?;
                for (entry, info) in archive.entries.iter().enumerate() {
                    let key = normalize_path(&info.name)?;
                    if assets
                        .insert(
                            key.clone(),
                            AssetOrigin::Archive {
                                archive: archives.len(),
                                entry,
                            },
                        )
                        .is_some()
                    {
                        return Err(Error::Invalid(format!("конфликт архивов для {key}")));
                    }
                }
                archives.push(archive);
            }
        }
        if archives.is_empty() {
            return Err(Error::Invalid("в каталоге игры не найдены .gut".into()));
        }
        let mut loose = BTreeMap::new();
        index_loose(&data_dir, &data_dir, &mut loose, false, 0)?;
        assets.extend(loose);
        if let Some(root) = overrides {
            let mut replacements = BTreeMap::new();
            index_loose(root, root, &mut replacements, true, 0)?;
            assets.extend(replacements);
        }
        Ok(Self { archives, assets })
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.assets.keys().map(String::as_str)
    }

    pub fn origin(&self, path: &str) -> Result<&AssetOrigin> {
        let key = normalize_path(path)?;
        self.assets
            .get(&key)
            .ok_or_else(|| Error::NotFound(path.into()))
    }

    pub fn source_label(&self, path: &str) -> Result<String> {
        Ok(match self.origin(path)? {
            AssetOrigin::Archive { archive, .. } => {
                format!("GUT: {}", self.archives[*archive].name)
            }
            AssetOrigin::Loose(_) => "Файл из каталога Data".into(),
            AssetOrigin::Override(_) => "Наш пакет замен".into(),
        })
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        match self.origin(path)? {
            AssetOrigin::Archive { archive, entry } => self.archives[*archive].read_entry(*entry),
            AssetOrigin::Loose(file) | AssetOrigin::Override(file) => {
                if fs::metadata(file)?.len() > 512 * 1024 * 1024 {
                    return Err(Error::Invalid(
                        "один ресурс больше предела прототипа 512 МиБ".into(),
                    ));
                }
                Ok(fs::read(file)?)
            }
        }
    }

    /// SMF обычно хранит только имя BMP. Известные каталоги имеют приоритет;
    /// при неоднозначном поиске по имени возвращаем ошибку, а не случайный файл.
    pub fn resolve_texture(&self, model: &str, texture: &str) -> Result<Option<String>> {
        if texture.is_empty() {
            return Ok(None);
        }
        let texture = normalize_path(texture)?;
        let model = normalize_path(model)?;
        let directory = model.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let relative = if directory.is_empty() {
            texture.clone()
        } else {
            format!("{directory}/{texture}")
        };
        for candidate in [
            texture.clone(),
            relative,
            format!("textures/shared/{texture}"),
        ] {
            let candidate = normalize_path(&candidate)?;
            if self.assets.contains_key(&candidate) {
                return Ok(Some(candidate));
            }
        }
        let basename = texture.rsplit('/').next().expect("непустой путь");
        let mut matches = self.paths().filter(|path| {
            path.starts_with("textures/") && path.rsplit('/').next() == Some(basename)
        });
        let first = matches.next().map(str::to_owned);
        if matches.next().is_some() {
            return Err(Error::Invalid(format!("неоднозначная текстура {texture}")));
        }
        Ok(first)
    }
}

fn index_loose(
    root: &Path,
    directory: &Path,
    assets: &mut BTreeMap<String, AssetOrigin>,
    improved: bool,
    depth: usize,
) -> Result<()> {
    if depth > 32 {
        return Err(Error::Invalid("слишком много вложенных каталогов".into()));
    }
    let mut files = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    files.sort_by_key(|entry| entry.path());
    for file in files {
        let file_type = file.file_type()?;
        let path = file.path();
        // Символические ссылки не должны незаметно вести к чужому каталогу.
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            index_loose(root, &path, assets, improved, depth + 1)?;
        } else if file_type.is_file()
            && !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gut"))
        {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| Error::Invalid("неверный корень ресурсов".into()))?;
            let key = normalize_path(&relative.to_string_lossy())?;
            let origin = if improved {
                AssetOrigin::Override(path)
            } else {
                AssetOrigin::Loose(path)
            };
            if assets.insert(key.clone(), origin).is_some() {
                return Err(Error::Invalid(format!("конфликт регистра для {key}")));
            }
        }
    }
    Ok(())
}
