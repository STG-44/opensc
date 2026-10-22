//! Чтение данных Shadow Company независимо от Bevy и графического устройства.
//! Начни изучение проекта с `gut.rs`, затем переходи к `catalog.rs`.

mod avi;
mod bmf;
mod catalog;
mod character;
mod gut;
mod help;
mod menu;
mod mission;
mod mtf;
mod pcx;
mod smf;
mod terrain;
mod text;

pub use avi::{AviAudio, AviMovie, PcmSamples};
pub use bmf::{BmfFrame, BmfKey, BmfMotion};
pub use catalog::{AssetCatalog, AssetOrigin};
pub use character::{CharacterBody, CharacterProfile};
pub use gut::{GutArchive, GutEntry};
pub use help::{HelpDefinition, HelpDefinitions};
pub use menu::{MenuButton, MenuFit, MenuLayout};
pub use mission::{
    CampaignActionType, CampaignEventType, MissionAction, MissionAi, MissionConversation,
    MissionDefinition, MissionDiagnostic, MissionObjective, MissionObjectiveLink, MissionPlayer,
    MissionResponse, MissionZone,
};
pub use mtf::{MtfMap, MtfObject};
pub use pcx::PcxImage;
pub use smf::{SmfBounds, SmfFace, SmfMesh, SmfModel, SmfNode, SmfVersion, SmfVertex};
pub use terrain::{TerrainConfig, TerrainGrid};
pub use text::{ConfigLine, config_lines};

use std::{fmt, io};

/// Ошибка — обычное значение. Вызывающий код решает, как показать её пользователю.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid(String),
    NotFound(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Invalid(message) => write!(f, "Некорректные данные: {message}"),
            Self::NotFound(path) => write!(f, "Ресурс не найден: {path}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Архивы используют Windows-разделители и нечувствительные к регистру имена.
/// Нормализация нужна и на Linux, где обычные файловые пути ведут себя иначе.
pub fn normalize_path(path: &str) -> Result<String> {
    let path = path.replace('\\', "/");
    if path.starts_with('/') || path.contains(':') || path.contains('\0') {
        return Err(Error::Invalid(format!("недопустимый путь {path:?}")));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(Error::Invalid("путь выходит за каталог ресурсов".into())),
            _ => parts.push(part.to_lowercase()),
        }
    }
    if parts.is_empty() {
        return Err(Error::Invalid("пустой путь ресурса".into()));
    }
    Ok(parts.join("/"))
}
