//! Наши настройки хранятся отдельно от установленного оригинала.
use std::{
    error::Error,
    path::{Path, PathBuf},
};
#[derive(bevy::prelude::Resource, Debug, Clone, PartialEq)]
pub struct Preferences {
    pub english: bool,
    pub fullscreen: bool,
    pub window_size: [u32; 2],
    pub text_scale: f32,
    pub animate_background: bool,
    pub master_volume: f32,
    pub camera_speed: f32,
    pub edge_scroll: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            english: false,
            fullscreen: false,
            window_size: [1280, 720],
            text_scale: 1.0,
            animate_background: true,
            master_volume: 1.0,
            camera_speed: 1.0,
            edge_scroll: true,
        }
    }
}
impl Preferences {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, Box<dyn Error>> {
        let version = value["version"].as_u64().ok_or("нет версии настроек")?;
        if version != 1 {
            return Err("неподдерживаемая версия настроек".into());
        }
        let english = match value["language"].as_str() {
            Some("ru") => false,
            Some("en") => true,
            _ => return Err("неверный язык настроек".into()),
        };
        let text_scale = value["text_scale"].as_f64().ok_or("нет размера текста")? as f32;
        if ![1.0, 1.25, 1.5].contains(&text_scale) {
            return Err("неверный размер текста".into());
        }
        let window_size = match value.get("window_size") {
            None => [1280, 720], // Настройки раннего прототипа без этого поля.
            Some(v) => {
                let values = v
                    .as_array()
                    .filter(|v| v.len() == 2)
                    .ok_or("неверное разрешение")?;
                let mut size = [0; 2];
                for (i, value) in values.iter().enumerate() {
                    size[i] = u32::try_from(value.as_u64().ok_or("неверное разрешение")?)?;
                }
                validate_window_size(size)?;
                size
            }
        };
        let setting =
            |name: &str, default: f32, min: f32, max: f32| -> Result<f32, Box<dyn Error>> {
                let n = match value.get(name) {
                    None => default,
                    Some(v) => v.as_f64().ok_or("неверный числовой параметр")? as f32,
                };
                if !n.is_finite() || !(min..=max).contains(&n) {
                    return Err(format!("неверный параметр {name}").into());
                }
                Ok(n)
            };
        Ok(Self {
            english,
            text_scale,
            window_size,
            fullscreen: value["fullscreen"].as_bool().ok_or("нет режима окна")?,
            animate_background: value["animate_background"]
                .as_bool()
                .ok_or("нет режима фонов")?,
            master_volume: setting("master_volume", 1.0, 0.0, 1.0)?,
            camera_speed: setting("camera_speed", 1.0, 0.5, 2.0)?,
            edge_scroll: match value.get("edge_scroll") {
                None => true,
                Some(v) => v.as_bool().ok_or("неверный режим прокрутки")?,
            },
        })
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"version":1,"language":if self.english {"en"} else {"ru"},"fullscreen":self.fullscreen,"window_size":self.window_size,"text_scale":self.text_scale,"animate_background":self.animate_background,"master_volume":self.master_volume,"camera_speed":self.camera_speed,"edge_scroll":self.edge_scroll})
    }
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }
        if std::fs::metadata(path)?.len() > 65536 {
            return Err("файл настроек слишком велик".into());
        }
        Self::from_json(&serde_json::from_slice(&std::fs::read(path)?)?)
    }
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        Self::from_json(&self.json())?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let filename = path.file_name().ok_or("неверный путь настроек")?;
        let mut temporary = filename.to_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = parent.join(temporary);
        let result = (|| {
            std::fs::write(&temporary, serde_json::to_vec_pretty(&self.json())?)?;
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}
pub fn validate_window_size(size: [u32; 2]) -> Result<(), Box<dyn Error>> {
    if !(1280..=16384).contains(&size[0]) || !(720..=16384).contains(&size[1]) {
        return Err("разрешение должно быть от 1280×720 до 16384×16384".into());
    }
    Ok(())
}
pub fn default_path() -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("APPDATA не задан; укажи --settings")?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME не задан; укажи --settings")?
        .join("Library/Application Support");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or("каталог настроек неизвестен; укажи --settings")?;
    Ok(base.join("shadow-company-bevy/settings.json"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_settings_and_roundtrips_supported_values() {
        let p = Preferences {
            english: true,
            fullscreen: true,
            window_size: [3440, 1440],
            text_scale: 1.5,
            animate_background: false,
            master_volume: 0.5,
            camera_speed: 1.5,
            edge_scroll: false,
        };
        assert_eq!(Preferences::from_json(&p.json()).unwrap(), p);
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("language", serde_json::json!("xx")),
            ("text_scale", serde_json::json!(99)),
            ("fullscreen", serde_json::json!("true")),
            ("window_size", serde_json::json!([800, 600])),
            ("window_size", serde_json::json!([1920])),
            ("window_size", serde_json::json!([1920, 1080, 1])),
            ("window_size", serde_json::json!([99999999999_u64, 1080])),
            ("master_volume", serde_json::json!(1.1)),
            ("master_volume", serde_json::json!("quiet")),
            ("camera_speed", serde_json::json!(0.1)),
            ("edge_scroll", serde_json::json!(1)),
        ] {
            let mut bad = p.json();
            bad[key] = value;
            assert!(Preferences::from_json(&bad).is_err());
        }
    }
    #[test]
    fn old_settings_keep_original_audio_and_camera_defaults() {
        let mut json = Preferences::default().json();
        let object = json.as_object_mut().unwrap();
        for key in ["master_volume", "camera_speed", "edge_scroll"] {
            object.remove(key);
        }
        assert_eq!(
            Preferences::from_json(&json).unwrap(),
            Preferences::default()
        );
    }
    #[test]
    fn saves_atomically_to_unicode_path_and_replaces_existing_settings() {
        let root = std::env::temp_dir().join(format!("shadow-menu-{}", std::process::id()));
        let path = root.join("Настройки игры/settings.json");
        assert_eq!(Preferences::load(&path).unwrap(), Preferences::default());
        Preferences::default().save(&path).unwrap();
        let p = Preferences {
            english: true,
            ..Default::default()
        };
        p.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), p);
        std::fs::write(&path, b"broken").unwrap();
        assert!(Preferences::load(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
