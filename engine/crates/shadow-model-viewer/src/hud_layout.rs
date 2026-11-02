//! Поднабор WINDOW_BASE нижней панели. Координаты и UV читаются из игры.
use bevy::prelude::*;
use shadow_formats::ConfigLine;
use std::{collections::BTreeMap, error::Error};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Debug)]
pub struct Quad {
    pub texture: String,
    pub rect: Rect,
    pub uv: Option<Rect>,
    pub canvas: Vec2,
    pub tile_x: bool,
    pub colors: [Color; 4],
}
#[derive(Debug)]
pub struct Advice {
    pub id: String,
    pub rect: Rect,
    pub frame: Option<usize>,
}
#[derive(Debug)]
pub struct Layout {
    pub quads: Vec<Quad>,
    pub advice: Vec<Advice>,
    pub regions: Vec<Rect>,
    pub face: Rect,
    pub stride: f32,
}
impl Layout {
    pub fn advice(&self, id: &str) -> &Advice {
        self.advice
            .iter()
            .find(|a| a.id == id)
            .expect("validated HUD advice")
    }
}
fn number(s: &str, vars: &BTreeMap<String, f32>) -> Result<f32> {
    let mut parts = s.split(',');
    let name = parts.next().ok_or("пустая координата HUD")?;
    let mut n = match vars.get(name) {
        Some(n) => *n,
        None => name.parse::<f32>()?,
    };
    for offset in parts {
        n += offset.parse::<f32>()?;
    }
    if !n.is_finite() || n.abs() > 100_000.0 {
        return Err("координата HUD вне диапазона".into());
    }
    Ok(n)
}
fn rect(w: &[String], vars: &BTreeMap<String, f32>) -> Result<Rect> {
    if w.len() < 4 {
        return Err("неполный прямоугольник HUD".into());
    }
    let p = Vec2::new(number(&w[0], vars)?, number(&w[1], vars)?);
    let size = Vec2::new(number(&w[2], vars)?, number(&w[3], vars)?);
    if size.min_element() <= 0.0 {
        return Err("пустой прямоугольник HUD".into());
    }
    Ok(Rect::from_corners(p, p + size))
}
/// Повторно вычисляется только при изменении размеров окна, без округления в 4:3.
pub fn parse(lines: &[ConfigLine], screen: Vec2) -> Result<Layout> {
    if !screen.is_finite() || screen.min_element() < 720.0 {
        return Err("HUD: нужен современный экран HD или выше".into());
    }
    let mut vars = BTreeMap::from([
        ("%screen_dx".into(), screen.x),
        ("%screen_dy".into(), screen.y),
    ]);
    let mut result = Layout {
        quads: Vec::new(),
        advice: Vec::new(),
        regions: Vec::new(),
        face: Rect::default(),
        stride: 0.0,
    };
    let mut i = 0;
    while i < lines.len() {
        let w = &lines[i].words;
        match w[0].as_str() {
            "DEFINE_USER_IVAR" if w.len() == 3 => {
                let n = number(&w[2], &vars)?;
                if vars.insert(w[1].clone(), n).is_some() {
                    return Err("повтор переменной HUD".into());
                }
            }
            "MODIFY_USER_IVAR" if w.len() == 4 => {
                let rhs = number(&w[3], &vars)?;
                let lhs = vars.get_mut(&w[1]).ok_or("неизвестная переменная HUD")?;
                *lhs = match w[2].as_str() {
                    "+" => *lhs + rhs,
                    "-" => *lhs - rhs,
                    "*" => *lhs * rhs,
                    "/" if rhs != 0.0 => *lhs / rhs,
                    _ => return Err("неподдержанная операция HUD".into()),
                };
                if !lhs.is_finite() {
                    return Err("нечисловая переменная HUD".into());
                }
            }
            "DEFINE_WITHIN_REGION" if w.len() == 6 && w[1] == "RECT" => {
                result.regions.push(rect(&w[2..], &vars)?)
            }
            "DEFINE_BUTTON_ADVICE" if w.len() == 6 || w.len() == 9 => {
                if result.advice.iter().any(|a| a.id == w[1]) {
                    return Err("повтор кнопки HUD".into());
                }
                if w.len() == 9 && (w[6] != "7" || w[7] != "-1") {
                    return Err("неподдержанный атлас кнопки HUD".into());
                }
                result.advice.push(Advice {
                    id: w[1].clone(),
                    rect: rect(&w[2..], &vars)?,
                    frame: if w.len() == 9 {
                        Some(w[8].parse()?)
                    } else {
                        None
                    },
                });
            }
            "WINDOW_BASE_GEOMETRY" => {
                let end = lines[i + 1..]
                    .iter()
                    .position(|l| l.words[0] == "WINDOW_BASE_GEOMETRY")
                    .map_or(lines.len(), |n| i + 1 + n);
                let mut texture = None;
                let mut canvas = Vec2::ZERO;
                let mut vertices = Vec::new();
                let mut polygons = Vec::new();
                for line in &lines[i + 1..end] {
                    let g = &line.words;
                    match g[0].as_str() {
                        "GEOMETRY_TEXTURE" if g.len() == 2 => texture = Some(g[1].clone()),
                        "GEOMETRY_TEXTURE_PACK_DX" | "GEOMETRY_TEXTURE_PACK_DY" if g.len() == 2 => {
                            let v = number(&g[1], &vars)?;
                            let v = if v == 255.0 { 256.0 } else { v };
                            if g[0].ends_with("DX") {
                                canvas.x = v;
                            } else {
                                canvas.y = v;
                            }
                        }
                        "GEOMETRY_VERTEX" if g.len() == 7 || g.len() == 9 => {
                            let p = Vec2::new(number(&g[1], &vars)?, number(&g[2], &vars)?);
                            let c = [
                                number(&g[3], &vars)?,
                                number(&g[4], &vars)?,
                                number(&g[5], &vars)?,
                                number(&g[6], &vars)?,
                            ];
                            if c.iter().any(|c| !(0.0..=1.0).contains(c)) {
                                return Err("неверный цвет HUD".into());
                            }
                            let uv = if g.len() == 9 {
                                Some((
                                    if g[7] == "AUTO" {
                                        None
                                    } else {
                                        Some(number(&g[7], &vars)?)
                                    },
                                    number(&g[8], &vars)?,
                                ))
                            } else {
                                None
                            };
                            vertices.push((p, Color::srgba(c[0], c[1], c[2], c[3]), uv));
                        }
                        "GEOMETRY_POLYGON" if g.len() == 4 => {
                            polygons.push([g[1].parse::<usize>()?, g[2].parse()?, g[3].parse()?])
                        }
                        "GEOMETRY_VERTICES" if g.get(1).is_some_and(|s| s == "4") => {}
                        "GEOMETRY_POLYGONS" if g.get(1).is_some_and(|s| s == "2") => {}
                        "GEOMETRY_BLEND_MODE" | "GEOMETRY_BILINEAR_FILTERING" => {}
                        _ => return Err(format!("неподдержанная геометрия HUD: {}", g[0]).into()),
                    }
                }
                if vertices.len() != 4
                    || polygons != [[0, 1, 2], [2, 3, 0]]
                    || canvas.min_element() <= 0.0
                {
                    return Err("HUD: нужны два треугольника прямоугольника".into());
                }
                let p: Vec<_> = vertices.iter().map(|v| v.0).collect();
                if p[0].x != p[3].x
                    || p[1].x != p[2].x
                    || p[0].y != p[1].y
                    || p[2].y != p[3].y
                    || p[2].x <= p[0].x
                    || p[2].y <= p[0].y
                {
                    return Err("непрямоугольная геометрия HUD".into());
                }
                let tile_x = vertices.iter().all(|v| v.2.is_some_and(|u| u.0.is_none()));
                let uv = match (vertices[0].2, vertices[2].2) {
                    (None, None) => None,
                    (Some(a), Some(b)) if tile_x => Some(Rect::new(0.0, a.1, canvas.x, b.1)),
                    (Some((Some(x), y)), Some((Some(xx), yy))) => Some(Rect::new(x, y, xx, yy)),
                    _ => return Err("неподдержанные UV HUD".into()),
                };
                if let Some(uv) = uv
                    && (uv.min.min_element() < 0.0
                        || uv.width() <= 0.0
                        || uv.height() <= 0.0
                        || uv.max.x > canvas.x
                        || uv.max.y > canvas.y)
                {
                    return Err("UV HUD вне атласа".into());
                }
                result.quads.push(Quad {
                    texture: texture.ok_or("нет текстуры HUD")?,
                    rect: Rect::from_corners(p[0], p[2]),
                    uv,
                    canvas,
                    tile_x,
                    colors: vertices
                        .iter()
                        .map(|v| v.1)
                        .collect::<Vec<_>>()
                        .try_into()
                        .unwrap(),
                });
                i = end;
                continue;
            }
            "WINDOW_BASE"
            | "WINDOW_BASE_DX"
            | "WINDOW_BASE_DY"
            | "WINDOW_BASE_RELOAD_ON_MODE_SWITCH" => {}
            _ => return Err(format!("неподдержанное поле HUD: {}", w[0]).into()),
        }
        i += 1;
    }
    let field = |s: &str| vars.get(s).copied().ok_or("нет геометрии портретов HUD");
    result.face = Rect::from_corners(
        Vec2::new(
            field("$commando_panel_entries_start_x")?,
            field("$commando_panel_entries_start_y")?,
        ),
        Vec2::new(
            field("$commando_panel_entries_start_x")? + field("$commando_panel_entry_dx")?,
            field("$commando_panel_entries_start_y")? + field("$commando_panel_entry_dy")?,
        ),
    );
    result.stride = field("$commando_panel_entry_offset_x")?;
    for name in [
        "r_radar",
        "r_clock",
        "b_halt",
        "b_stand",
        "b_crouch",
        "b_prone",
        "b_inventory",
        "b_track",
        "b_menu",
    ] {
        if !result.advice.iter().any(|a| a.id == name) {
            return Err(format!("нет кнопки HUD {name}").into());
        }
    }
    if result.regions.len() != 3 || result.quads.is_empty() {
        return Err("нет областей HUD".into());
    }
    Ok(result)
}

/// У SPRITEFRAME края включительные; XRUN портретов задаёт ширину отдельно.
pub fn button_frames(lines: &[ConfigLine]) -> Result<Vec<Rect>> {
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.words.first().is_some_and(|s| s == "ANIMSPRITE3D")
                && l.words.get(1).is_some_and(|s| s == "interface_elements_7")
        })
        .collect();
    let [(start, header)] = starts.as_slice() else {
        return Err("нужен один interface_elements_7".into());
    };
    if header.words.len() != 7
        || header.words[2] != "simu1_ck.bmp"
        || header.words[3..5] != ["256", "256"]
    {
        return Err("неподдержанный атлас команд HUD".into());
    }
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.words[0] == "ENDDEF")
        .ok_or("нет ENDDEF HUD")?
        + start
        + 1;
    let vars = BTreeMap::new();
    let mut frames = Vec::new();
    for line in &lines[start + 1..end] {
        let w = &line.words;
        match w[0].as_str() {
            "FRAMEDESCRIPTOR" if w.len() == 4 && w[1..] == ["16", "16", "0"] => {}
            "SPRITEFRAME" if w.len() == 5 => {
                let r = Rect::new(
                    number(&w[1], &vars)?,
                    number(&w[2], &vars)?,
                    number(&w[3], &vars)? + 1.0,
                    number(&w[4], &vars)? + 1.0,
                );
                if r.min.min_element() < 0.0
                    || r.max.max_element() > 256.0
                    || r.width() <= 0.0
                    || r.height() <= 0.0
                {
                    return Err("кадр команды вне атласа HUD".into());
                }
                frames.push(r);
            }
            _ => return Err("неподдержанный кадр команды HUD".into()),
        }
    }
    if frames.len() != 16 {
        return Err("нужны 16 кадров команд HUD".into());
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<ConfigLine> {
        let mut s = String::from(
            "WINDOW_BASE hud\nDEFINE_USER_IVAR $right %screen_dx\nMODIFY_USER_IVAR $right - 167\nDEFINE_USER_IVAR $commando_panel_entries_start_x 195\nDEFINE_USER_IVAR $commando_panel_entries_start_y 96\nDEFINE_USER_IVAR $commando_panel_entry_dx 42\nDEFINE_USER_IVAR $commando_panel_entry_dy 64\nDEFINE_USER_IVAR $commando_panel_entry_offset_x 50\nDEFINE_WITHIN_REGION RECT 0 6 155 186\nDEFINE_WITHIN_REGION RECT 186 70 %screen_dx,-354 122\nDEFINE_WITHIN_REGION RECT $right 0 167 192\n",
        );
        for (i, id) in [
            "r_radar",
            "r_clock",
            "b_halt",
            "b_stand",
            "b_crouch",
            "b_prone",
            "b_inventory",
            "b_track",
            "b_menu",
        ]
        .iter()
        .enumerate()
        {
            s.push_str(&format!(
                "DEFINE_BUTTON_ADVICE {id} %screen_dx,-142 {} 33 33\n",
                i * 10
            ));
        }
        s.push_str("WINDOW_BASE_GEOMETRY\nGEOMETRY_TEXTURE simu3_ck.bmp\nGEOMETRY_TEXTURE_PACK_DX 255\nGEOMETRY_TEXTURE_PACK_DY 255\nGEOMETRY_VERTICES 4\nGEOMETRY_VERTEX 257 69 1 1 1 1 AUTO 30\nGEOMETRY_VERTEX %screen_dx,-454 69 1 1 1 1 AUTO 30\nGEOMETRY_VERTEX %screen_dx,-454 83 1 1 1 1 AUTO 44\nGEOMETRY_VERTEX 257 83 1 1 1 1 AUTO 44\nGEOMETRY_POLYGONS 2\nGEOMETRY_POLYGON 0 1 2\nGEOMETRY_POLYGON 2 3 0\n");
        shadow_formats::config_lines(s.as_bytes()).unwrap()
    }
    #[test]
    fn widescreen_moves_right_edge_and_extends_only_middle() {
        let a = parse(&fixture(), Vec2::new(1600.0, 900.0)).unwrap();
        let b = parse(&fixture(), Vec2::new(2100.0, 900.0)).unwrap();
        assert_eq!(a.face, b.face);
        assert_eq!(a.face.size(), Vec2::new(42.0, 64.0));
        assert_eq!(
            b.advice("b_menu").rect.min.x - a.advice("b_menu").rect.min.x,
            500.0
        );
        assert_eq!(b.quads[0].rect.width() - a.quads[0].rect.width(), 500.0);
        assert_eq!(b.regions[0], a.regions[0]);
        assert_eq!(b.regions[2].max.x, 2100.0);
        assert!(a.quads[0].tile_x);
        assert_eq!(a.quads[0].uv, Some(Rect::new(0.0, 30.0, 256.0, 44.0)));
    }
    #[test]
    fn rejects_invalid_arithmetic_geometry_and_repeated_buttons() {
        for replacement in [
            "MODIFY_USER_IVAR $right / 0",
            "MODIFY_USER_IVAR $missing - 1",
        ] {
            let mut lines = fixture();
            lines[2] = shadow_formats::config_lines(replacement.as_bytes())
                .unwrap()
                .remove(0);
            assert!(parse(&lines, Vec2::new(1600.0, 900.0)).is_err());
        }
        let mut lines = fixture();
        lines.push(
            lines
                .iter()
                .find(|l| l.words[0] == "DEFINE_BUTTON_ADVICE")
                .unwrap()
                .clone(),
        );
        assert!(parse(&lines, Vec2::new(1600.0, 900.0)).is_err());
        let mut lines = fixture();
        lines
            .iter_mut()
            .find(|l| l.words[0] == "GEOMETRY_POLYGON")
            .unwrap()
            .words[1] = "999".into();
        assert!(parse(&lines, Vec2::new(1600.0, 900.0)).is_err());
    }
    #[test]
    fn command_frames_have_inclusive_edges_and_must_fit_canvas() {
        let mut s = String::from(
            "ANIMSPRITE3D interface_elements_7 simu1_ck.bmp 256 256 1.0 1\nFRAMEDESCRIPTOR 16 16 0\n",
        );
        s.push_str(&"SPRITEFRAME 13 110 45 143\n".repeat(16));
        s.push_str("ENDDEF\n");
        let mut lines = shadow_formats::config_lines(s.as_bytes()).unwrap();
        assert_eq!(
            button_frames(&lines).unwrap()[0].size(),
            Vec2::new(33.0, 34.0)
        );
        lines[2].words[3] = "256".into();
        assert!(button_frames(&lines).is_err());
        lines.remove(2);
        assert!(button_frames(&lines).is_err());
    }
}
