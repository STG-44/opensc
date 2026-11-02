//! Базовая разметка главного меню из WINDOW_BASE, без графического движка.
use crate::{Error, Result, config_lines, normalize_path};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct MenuButton {
    pub id: String,
    pub position: [f32; 2],
}
#[derive(Debug, Clone)]
pub struct MenuLayout {
    pub size: [f32; 2],
    pub text_offset: [f32; 2],
    pub shadow_offset: [f32; 2],
    pub buttons: Vec<MenuButton>,
    pub backgrounds: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuFit {
    pub scale: f32,
    pub origin: [f32; 2],
    pub size: [f32; 2],
}
impl MenuLayout {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let lines = config_lines(bytes)?;
        let mut size = [None, None];
        let mut text_offset = [0.0; 2];
        let mut shadow_offset = [43.0, 13.0];
        let mut buttons = Vec::new();
        let mut backgrounds = Vec::new();
        let mut ids = BTreeSet::new();
        let number = |words: &[String], index: usize, line: usize| -> Result<f32> {
            let value = words
                .get(index)
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or_else(|| Error::Invalid(format!("меню: неверное число в строке {line}")))?;
            Ok(value)
        };
        for line in lines {
            let w = &line.words;
            match w[0].as_str() {
                "WINDOW_BASE_DX" | "WINDOW_BASE_DY" => {
                    let axis = usize::from(w[0] == "WINDOW_BASE_DY");
                    let n = number(w, 1, line.number)?;
                    if size[axis].is_some() || !(1.0..=8192.0).contains(&n) {
                        return Err(Error::Invalid("меню: неверный или повторный размер".into()));
                    }
                    size[axis] = Some(n);
                }
                "DEFINE_USER_IVAR"
                    if w.get(1)
                        .is_some_and(|s| s == "button_offset_x" || s == "button_offset_y") =>
                {
                    let axis = usize::from(w[1] == "button_offset_y");
                    text_offset[axis] = number(w, 2, line.number)?;
                }
                "DEFINE_USER_IVAR"
                    if w.get(1)
                        .is_some_and(|s| s == "shadow_offset_x" || s == "shadow_offset_y") =>
                {
                    let axis = usize::from(w[1] == "shadow_offset_y");
                    shadow_offset[axis] = number(w, 2, line.number)?;
                    if shadow_offset[axis].abs() > 8192.0 {
                        return Err(Error::Invalid("меню: неверное смещение тени".into()));
                    }
                }
                "DEFINE_BUTTON_ADVICE" if w.get(1).is_some_and(|s| s.starts_with("b_")) => {
                    let id = w[1].clone();
                    if !ids.insert(id.clone()) {
                        return Err(Error::Invalid(format!("меню: повтор кнопки {id}")));
                    }
                    buttons.push(MenuButton {
                        id,
                        position: [number(w, 2, line.number)?, number(w, 3, line.number)?],
                    });
                }
                "GEOMETRY_JPG_NAME" => {
                    let name = w
                        .get(1)
                        .ok_or_else(|| Error::Invalid("меню: отсутствует имя фона".into()))?;
                    let path = normalize_path(&format!("textures/interface/{name}.jpg"))?;
                    if !backgrounds.contains(&path) {
                        backgrounds.push(path);
                    }
                }
                _ => {}
            }
        }
        let size = [
            size[0].ok_or_else(|| Error::Invalid("меню: нет ширины".into()))?,
            size[1].ok_or_else(|| Error::Invalid("меню: нет высоты".into()))?,
        ];
        if backgrounds.is_empty() || !ids.contains("b_training") || !ids.contains("b_exit") {
            return Err(Error::Invalid("меню: нужны фон, обучение и выход".into()));
        }
        if buttons.iter().any(|b| {
            b.position.iter().enumerate().any(|(i, v)| {
                *v < 0.0
                    || *v >= size[i]
                    || *v + text_offset[i] < 0.0
                    || *v + text_offset[i] >= size[i]
            })
        }) {
            return Err(Error::Invalid(
                "меню: кнопка выходит за базовый экран".into(),
            ));
        }
        // Упорядочиваем сверху вниз независимо от порядка строк в конфигурации.
        buttons.sort_by(|a, b| a.position[1].total_cmp(&b.position[1]));
        Ok(Self {
            size,
            text_offset,
            shadow_offset,
            buttons,
            backgrounds,
        })
    }
    /// Размер исходника — система координат импорта, а не разрешение экрана.
    /// Интерфейс занимает всё окно; высота задаёт равномерный масштаб элементов.
    pub fn fit(&self, width: f32, height: f32) -> Option<MenuFit> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let scale = height / self.size[1];
        Some(MenuFit {
            scale,
            size: [width, height],
            origin: [0.0, 0.0],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(width: u32, x: u32) -> String {
        format!(
            "WINDOW_BASE main_menu\nWINDOW_BASE_DX {width}\nWINDOW_BASE_DY 480\nDEFINE_USER_IVAR button_offset_x 30\nDEFINE_USER_IVAR button_offset_y 5\nDEFINE_BUTTON_ADVICE view_container 0 0 {width} 480\nDEFINE_BUTTON_ADVICE b_exit {} 320 0 0\nDEFINE_BUTTON_ADVICE b_training {x} 160 0 0\nGEOMETRY_JPG_NAME frame5\nGEOMETRY_JPG_NAME frame1\n",
            x - 20
        )
    }
    #[test]
    fn reads_original_and_hd_positions_without_hardcoded_shift() {
        for (width, x) in [(640, 315), (854, 529)] {
            let m = MenuLayout::from_bytes(fixture(width, x).as_bytes()).unwrap();
            assert_eq!(m.size, [width as f32, 480.0]);
            assert_eq!(m.text_offset, [30.0, 5.0]);
            assert_eq!(m.shadow_offset, [43.0, 13.0]);
            assert_eq!(m.buttons[0].id, "b_training");
            assert_eq!(m.buttons[0].position, [x as f32, 160.0]);
            assert_eq!(
                m.backgrounds,
                [
                    "textures/interface/frame5.jpg",
                    "textures/interface/frame1.jpg"
                ]
            );
        }
    }
    #[test]
    fn shadow_position_comes_from_the_selected_resource_layer() {
        let s = fixture(854, 529)
            + "DEFINE_USER_IVAR shadow_offset_x 51\nDEFINE_USER_IVAR shadow_offset_y 18\n";
        let m = MenuLayout::from_bytes(s.as_bytes()).unwrap();
        assert_eq!(m.shadow_offset, [51.0, 18.0]);
        for value in ["NaN", "inf", "8193"] {
            assert!(
                MenuLayout::from_bytes(
                    s.replace("shadow_offset_x 51", &format!("shadow_offset_x {value}"))
                        .as_bytes()
                )
                .is_err()
            );
        }
    }
    #[test]
    fn fills_modern_windows_independently_of_imported_aspect_ratio() {
        let m = MenuLayout::from_bytes(fixture(640, 315).as_bytes()).unwrap();
        assert_eq!(
            m.fit(1920.0, 1080.0).unwrap(),
            MenuFit {
                scale: 2.25,
                size: [1920.0, 1080.0],
                origin: [0.0, 0.0]
            }
        );
        for [w, h] in [
            [1280.0, 720.0],
            [2560.0, 1440.0],
            [3840.0, 2160.0],
            [2560.0, 1080.0],
            [3440.0, 1440.0],
            [5120.0, 2160.0],
            [7680.0, 4320.0],
        ] {
            let fit = m.fit(w, h).unwrap();
            assert_eq!(fit.size, [w, h]);
            assert_eq!(fit.origin, [0.0, 0.0]);
            assert_eq!(fit.scale, h / 480.0);
        }
        assert!(m.fit(0.0, 900.0).is_none());
        assert!(m.fit(f32::NAN, 900.0).is_none());
    }
    #[test]
    fn rejects_ambiguous_broken_and_unsafe_menu_data() {
        let valid = fixture(640, 315);
        for s in [
            valid.replace("DX 640", "DX NaN"),
            valid.replace("DY 480", "DY -1"),
            valid.clone() + "WINDOW_BASE_DX 640\n",
            valid.clone() + "DEFINE_BUTTON_ADVICE b_exit 1 1 0 0\n",
            valid.replace("b_training 315", "b_training 900"),
            valid.replace("frame5", "../frame5"),
            valid.replace("b_exit", "missing_exit"),
        ] {
            assert!(MenuLayout::from_bytes(s.as_bytes()).is_err(), "{s}");
        }
    }
}
