//! Build an owned, visual-only override pack. No original mesh or texture is
//! copied: only attachment metadata, silhouettes and interaction fingerprints.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, SmfFace, SmfMesh, SmfModel, SmfVertex};
use std::{error::Error, f32::consts::TAU, fs, path::Path};

type V = [f32; 3];
mod structures;
const STEEL: &str = "textures/opensc-environment/galvanized.png";
const BARK: &str = "textures/opensc-environment/bark.png";
const LEAVES: &str = "textures/opensc-environment/leaves.png";
const TEXTURES: &[&str] = &[
    "bark.png",
    "leaves.png",
    "galvanized.png",
    "masonry.png",
    "concrete.png",
    "hummer-body-v1.png",
];
const FENCES: &[&str] = &[
    "alstfn-consertinafence",
    "alstfn-consertinafence_end",
    "alstfn-consertinafence_cut_end",
    "alstfn-consertinafencegate",
    "alstfn-barbfence",
    "alstfn-barbfence_cut_end",
];

pub fn build(
    catalog: &AssetCatalog,
    textures: &Path,
    output: &Path,
) -> Result<Value, Box<dyn Error>> {
    // Never overlay an existing directory, original installation or user pack.
    for name in TEXTURES {
        fs::metadata(textures.join(name))?;
    }
    let optics: Value = serde_json::from_slice(&fs::read(
        textures
            .parent()
            .ok_or("Missing optics directory")?
            .join("hummer-optics.json"),
    )?)?;
    if optics["texture_sha256"].as_str()
        != Some(&format!(
            "{:x}",
            Sha256::digest(fs::read(textures.join("hummer-body-v1.png"))?)
        ))
    {
        return Err("Hummer optics must be measured again after repainting the atlas".into());
    }
    let mut assets = Vec::new();
    for variant in 1..=3 {
        let name = format!("alsctr-south{variant}");
        assets.push((name, Some(variant)));
    }
    assets.extend(FENCES.iter().map(|s| (s.to_string(), None)));
    assets.extend(
        [
            "alvhqd-hummer",
            "alstwa-concrete01",
            "alstwa-concrete02",
            "alstwa-concrete03",
        ]
        .iter()
        .map(|name| (name.to_string(), None)),
    );
    let mut prepared = Vec::new();
    for (name, variant) in assets {
        let path = format!("models/{name}/{name}.smf");
        let source = catalog.read_base(&path)?;
        let mut model = SmfModel::from_bytes(&source)?;
        // These source profiles use untranslated Euler axes; do not guess how
        // to re-author an unknown skeleton or rotated collision helper.
        if model.nodes.iter().any(|n| n.flags.unwrap_or(0) & 4 == 0) {
            return Err(format!("Unsupported environment skeleton: {path}").into());
        }
        let mut wipers = None;
        if let Some(variant) = variant {
            tree(&mut model, variant)?;
        } else if name == "alvhqd-hummer" {
            wipers = Some(structures::hummer(&mut model, &optics)?);
        } else if name.starts_with("alstwa-concrete") {
            structures::wall(&mut model)?;
        } else {
            fence(&mut model)?;
        }
        let visual = encode(&model)?;
        let stats = json!({"path":path,"vertices":model.nodes.iter().flat_map(|n|&n.meshes).map(|m|m.vertices.len()).sum::<usize>(),
            "triangles":model.nodes.iter().flat_map(|n|&n.meshes).map(|m|m.faces.len()).sum::<usize>()});
        let mut metadata = json!({"version":1,"source_sha256":format!("{:x}",Sha256::digest(&source)),
            "visual_sha256":format!("{:x}",Sha256::digest(&visual))});
        if name == "alvhqd-hummer" {
            metadata["version"] = json!(2);
            metadata["render_base_parts"] = json!(["hummer-back01"]);
            metadata["wipers"] = wipers.unwrap();
            metadata["texture_overrides"] = json!({"textures/shared/hummer.bmp":{
                "texture":"textures/opensc-environment/hummer-body-v1.png",
                "glass_regions":[[0.503,0.282,0.589,0.416],[0.503,0.45,0.589,0.589],
                    [0.218,0.646,0.313,0.729],[0.375,0.646,0.465,0.729]]
            }});
        }
        prepared.push((path, visual, metadata, stats));
    }
    fs::create_dir(output)?;
    let texture_dir = output.join("textures/opensc-environment");
    fs::create_dir_all(&texture_dir)?;
    for name in TEXTURES {
        fs::copy(textures.join(name), texture_dir.join(name))?;
    }
    structures::swatches(&texture_dir)?;
    let mut report = Vec::new();
    for (path, visual, metadata, stats) in prepared {
        let target = output.join(&path);
        fs::create_dir_all(target.parent().ok_or("Missing model directory")?)?;
        fs::write(&target, visual)?;
        fs::write(
            output.join(format!("{path}.visual.json")),
            serde_json::to_vec_pretty(&metadata)?,
        )?;
        report.push(stats);
    }
    let report = json!({"version":1,"pack":"OpenSC environment prototype","models":report});
    fs::write(
        output.join("environment-pack.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

fn add(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V, s: f32) -> V {
    a.map(|v| v * s)
}
fn dot(a: V, b: V) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V) -> V {
    mul(a, 1.0 / dot(a, a).sqrt())
}
fn lerp(a: V, b: V, t: f32) -> V {
    add(a, mul(sub(b, a), t))
}
fn empty(name: &str, texture: &str) -> SmfMesh {
    SmfMesh {
        name: name.into(),
        texture: texture.into(),
        vertices: Vec::new(),
        faces: Vec::new(),
    }
}
fn vertex(m: &mut SmfMesh, p: V, n: V, uv: [f32; 2]) -> u32 {
    let id = m.vertices.len() as u32;
    m.vertices.push(SmfVertex {
        id,
        position: p,
        normal: Some(n),
        uv,
        color: u32::MAX,
        specular: 0,
    });
    id
}
fn face(m: &mut SmfMesh, a: u32, b: u32, c: u32) {
    // SMF stores clockwise faces; the renderer reverses them.
    m.faces.push(SmfFace {
        id: m.faces.len() as u32,
        indices: [a, c, b],
    });
}

/// Closed tapered cylinder, smooth radial normals and longitudinal bark UVs.
fn tube(m: &mut SmfMesh, a: V, b: V, r0: f32, r1: f32, sides: u32) {
    let delta = sub(b, a);
    let length = dot(delta, delta).sqrt();
    if length < 0.001 {
        return;
    }
    let axis = mul(delta, 1.0 / length);
    let u = unit(cross(
        axis,
        if axis[2].abs() < 0.9 {
            [0., 0., 1.]
        } else {
            [0., 1., 0.]
        },
    ));
    let v = cross(axis, u);
    let start = m.vertices.len() as u32;
    for (p, r, t) in [(a, r0, 0.0), (b, r1, length / 100.)] {
        for i in 0..=sides {
            let angle = TAU * i as f32 / sides as f32;
            let radial = add(mul(u, angle.cos()), mul(v, angle.sin()));
            let normal = unit(add(radial, mul(axis, (r0 - r1) / length)));
            vertex(
                m,
                add(p, mul(radial, r)),
                normal,
                [i as f32 / sides as f32, t],
            );
        }
    }
    let stride = sides + 1;
    for i in 0..sides {
        face(m, start + i, start + i + 1, start + stride + i);
        face(m, start + i + 1, start + stride + i + 1, start + stride + i);
    }
    let ca = vertex(m, a, mul(axis, -1.), [0.5, 0.5]);
    let cb = vertex(m, b, axis, [0.5, 0.5]);
    for i in 0..sides {
        face(m, ca, start + i + 1, start + i);
        face(m, cb, start + stride + i, start + stride + i + 1);
    }
}

fn bounds(mesh: &SmfMesh) -> (V, V) {
    mesh.vertices
        .iter()
        .fold(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]), |(a, b), v| {
            (
                std::array::from_fn(|i| a[i].min(v.position[i])),
                std::array::from_fn(|i| b[i].max(v.position[i])),
            )
        })
}

/// Sweep a tapered branch along a quadratic curve with shared rings. Unlike
/// joined straight cylinders this has no internal caps or angular elbows.
fn branch_curve(m: &mut SmfMesh, points: [V; 3], radii: [f32; 2], sides: u32) {
    let [a, control, b] = points;
    let [r0, r1] = radii;
    let steps = 7;
    let length =
        dot(sub(control, a), sub(control, a)).sqrt() + dot(sub(b, control), sub(b, control)).sqrt();
    let start = m.vertices.len() as u32;
    let mut previous_u: Option<V> = None;
    for ring in 0..=steps {
        let t = ring as f32 / steps as f32;
        let center = lerp(lerp(a, control, t), lerp(control, b, t), t);
        let axis = unit(lerp(sub(control, a), sub(b, control), t));
        let transported = previous_u.map(|u| sub(u, mul(axis, dot(u, axis))));
        let u = unit(
            transported
                .filter(|u| dot(*u, *u) > 1e-6)
                .unwrap_or_else(|| {
                    cross(
                        axis,
                        if axis[2].abs() < 0.95 {
                            [0., 0., 1.]
                        } else {
                            [0., 1., 0.]
                        },
                    )
                }),
        );
        previous_u = Some(u);
        let v = cross(axis, u);
        for i in 0..=sides {
            let angle = i as f32 / sides as f32 * TAU;
            let radial = add(mul(u, angle.cos()), mul(v, angle.sin()));
            vertex(
                m,
                add(center, mul(radial, r0 + (r1 - r0) * t)),
                unit(add(radial, mul(axis, (r0 - r1) / length))),
                [i as f32 / sides as f32, t * length / 100.],
            );
        }
    }
    let stride = sides + 1;
    for ring in 0..steps {
        for i in 0..sides {
            let p = start + ring * stride + i;
            face(m, p, p + 1, p + stride);
            face(m, p + 1, p + stride + 1, p + stride);
        }
    }
    for (center, normal, offset, reverse) in [
        (a, mul(unit(sub(control, a)), -1.), start, true),
        (b, unit(sub(b, control)), start + steps * stride, false),
    ] {
        let c = vertex(m, center, normal, [0.5, 0.5]);
        for i in 0..sides {
            if reverse {
                face(m, c, offset + i + 1, offset + i);
            } else {
                face(m, c, offset + i, offset + i + 1);
            }
        }
    }
}

/// Clip one diamond wire to the union of source panel silhouettes. Degenerate
/// and duplicate faces in legacy thin boxes cannot add overlapping wires.
fn wire_intervals(source: &SmfMesh, slope: f32, intercept: f32) -> Vec<(f32, f32)> {
    let mut intervals = Vec::new();
    for f in &source.faces {
        let p = f.indices.map(|i| source.vertices[i as usize].position);
        let area =
            (p[1][0] - p[0][0]) * (p[2][2] - p[0][2]) - (p[1][2] - p[0][2]) * (p[2][0] - p[0][0]);
        if area.abs() < 0.01 {
            continue;
        }
        let sign = area.signum();
        let mut low = f32::NEG_INFINITY;
        let mut high = f32::INFINITY;
        for i in 0..3 {
            let a = p[i];
            let b = p[(i + 1) % 3];
            let dx = b[0] - a[0];
            let dz = b[2] - a[2];
            // cross(edge, [x,slope*x+intercept]-a) >= 0
            let coefficient = sign * (dx * slope - dz);
            let constant = sign * (dx * (intercept - a[2]) + dz * a[0]);
            if coefficient.abs() < 1e-6 {
                if constant < -0.001 {
                    low = 1.;
                    high = 0.;
                    break;
                }
            } else if coefficient > 0. {
                low = low.max(-constant / coefficient);
            } else {
                high = high.min(-constant / coefficient);
            }
        }
        if high - low > 0.1 && low.is_finite() && high.is_finite() {
            intervals.push((low, high));
        }
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (a, b) in intervals {
        if let Some(last) = merged.last_mut().filter(|last| a <= last.1 + 0.01) {
            last.1 = last.1.max(b);
        } else {
            merged.push((a, b));
        }
    }
    merged
}

fn chain_link(source: &SmfMesh) -> SmfMesh {
    let mut m = empty(&source.name, STEEL);
    let (low, high) = bounds(source);
    let y = low[1];
    let spacing = 12.; // roughly 8.5 cm diamond openings
    for slope in [-1.0, 1.0] {
        let min = low[2] - (if slope > 0. { high[0] } else { -low[0] });
        let max = high[2] - (if slope > 0. { low[0] } else { -high[0] });
        for i in (min / spacing).floor() as i32..=(max / spacing).ceil() as i32 {
            let intercept = i as f32 * spacing;
            for (a, b) in wire_intervals(source, slope, intercept) {
                let yy = y + slope * 0.35;
                tube(
                    &mut m,
                    [a, yy, slope * a + intercept],
                    [b, yy, slope * b + intercept],
                    0.32,
                    0.32,
                    4,
                );
            }
        }
    }
    // Tension wires follow the same silhouettes, including the cut opening.
    for z in [low[2] + 1., high[2] - 1.] {
        for (a, b) in wire_intervals(source, 0., z) {
            tube(&mut m, [a, y, z], [b, y, z], 0.55, 0.55, 6);
        }
    }
    m
}

fn rails(source: &SmfMesh, panel: &SmfMesh, concertina: bool, gate_leaf: bool) -> SmfMesh {
    let mut m = empty(&source.name, STEEL);
    let (lo, hi) = bounds(source);
    let (pl, ph) = bounds(panel);
    let y = pl[1];
    let bottom = pl[2];
    let top = ph[2];
    for x in [lo[0] + 3.5, hi[0] - 3.5] {
        tube(&mut m, [x, y, bottom - 1.], [x, y, top + 5.], 3.2, 3.2, 8);
        tube(
            &mut m,
            [x, y, top + 5.],
            [x, y - 25., hi[2] - 3.],
            2.3,
            2.3,
            6,
        );
    }
    if gate_leaf {
        for z in [bottom, top] {
            tube(&mut m, [lo[0] + 3., y, z], [hi[0] - 3., y, z], 2.3, 2.3, 8);
        }
        tube(
            &mut m,
            [lo[0] + 3., y, bottom],
            [hi[0] - 3., y, top],
            1.4,
            1.4,
            6,
        );
    }
    if concertina && !gate_leaf {
        let start = lo[0] + 4.;
        let end = hi[0] - 4.;
        let radius = ((hi[1] - lo[1]) * 0.5).clamp(20., 45.);
        let center = hi[2] - radius;
        let turns = ((end - start) / 52.).round().max(1.);
        let segments = (turns * 32.) as usize;
        let point = |i: usize| {
            let t = i as f32 / segments as f32;
            let angle = t * turns * TAU;
            [
                start + (end - start) * t,
                y + radius * angle.cos(),
                center + radius * angle.sin(),
            ]
        };
        for i in 0..segments {
            let a = point(i);
            let b = point(i + 1);
            tube(&mut m, a, b, 0.5, 0.5, 4);
            if i % 8 == 0 {
                let outward = unit([0., a[1] - y, a[2] - center]);
                let tangent = unit(sub(b, a));
                for sign in [-1., 1.] {
                    let tip = add(a, add(mul(outward, sign * 3.8), mul(tangent, sign * 1.8)));
                    tube(&mut m, sub(a, mul(tangent, sign * 2.)), tip, 0.65, 0.05, 3);
                }
            }
        }
    } else {
        for row in 0..3 {
            let t = row as f32 / 2.;
            let yy = y - 25. * t;
            let zz = top + 5. + (hi[2] - top - 8.) * t;
            tube(
                &mut m,
                [lo[0] + 3., yy, zz],
                [hi[0] - 3., yy, zz],
                0.5,
                0.5,
                5,
            );
            let mut x = lo[0] + 14.;
            while x < hi[0] - 4. {
                for s in [-1., 1.] {
                    tube(
                        &mut m,
                        [x - 1., yy, zz],
                        [x + 2., yy + s * 2.8, zz + s * 2.8],
                        0.6,
                        0.05,
                        3,
                    );
                }
                x += 22.;
            }
        }
    }
    m
}

fn fence(model: &mut SmfModel) -> Result<(), Box<dyn Error>> {
    let concertina = model.name.to_ascii_lowercase().contains("consertina");
    for node in &mut model.nodes {
        if node.meshes.is_empty() {
            continue;
        }
        let old = std::mem::take(&mut node.meshes);
        let panel = old
            .iter()
            .find(|m| m.texture.eq_ignore_ascii_case("FenceDIFF_ck.BMP"))
            .ok_or("Fence panel not found")?;
        for source in &old {
            node.meshes.push(if source.name == panel.name {
                chain_link(source)
            } else {
                rails(source, panel, concertina, node.id == 291)
            });
        }
        if model.name.to_ascii_lowercase().ends_with("fencegate") && node.id != 291 {
            let (low, high) = bounds(panel);
            let intervals = wire_intervals(panel, 0., (low[2] + high[2]) * 0.5);
            if intervals.len() != 2 {
                return Err("Unsupported gate opening".into());
            }
            let rail = node
                .meshes
                .iter_mut()
                .find(|m| m.name != panel.name)
                .ok_or("Gate frame missing")?;
            for x in [intervals[0].1 - 2.5, intervals[1].0 + 2.5] {
                tube(rail, [x, low[1], low[2]], [x, low[1], high[2]], 2.5, 2.5, 8);
            }
        }
    }
    Ok(())
}

struct Random(u32);
impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f32 / 16777216.
    }
    fn signed(&mut self) -> f32 {
        self.next() * 2. - 1.
    }
}

/// A curved 2x2 spray, not a large flat billboard. Normals vary across the
/// patch; four UV quadrants provide distinct leaf silhouettes.
fn spray(m: &mut SmfMesh, center: V, size: f32, yaw: f32, tilt: f32, tile: u32) {
    let u = [yaw.cos(), yaw.sin(), 0.];
    let v = [-yaw.sin() * tilt.cos(), yaw.cos() * tilt.cos(), tilt.sin()];
    let n = unit(cross(u, v));
    let start = m.vertices.len() as u32;
    for y in 0..3 {
        for x in 0..3 {
            let a = x as f32 - 1.;
            let b = y as f32 - 1.;
            let bend = (1. - a * a) * 0.13 + (1. - b * b) * 0.10;
            let p = add(
                center,
                mul(
                    add(add(mul(u, a * 0.5), mul(v, b * 0.5)), mul(n, bend)),
                    size,
                ),
            );
            let normal = unit(add(n, add(mul(u, a * 0.52), mul(v, b * 0.40))));
            let uv = [
                (tile % 2) as f32 * 0.5 + 0.008 + x as f32 * 0.242,
                (tile / 2) as f32 * 0.5 + 0.008 + y as f32 * 0.242,
            ];
            vertex(m, p, normal, uv);
        }
    }
    for y in 0..2 {
        for x in 0..2 {
            let a = start + y * 3 + x;
            face(m, a, a + 1, a + 3);
            face(m, a + 1, a + 4, a + 3);
        }
    }
}

fn tree(model: &mut SmfModel, variant: u32) -> Result<(), Box<dyn Error>> {
    let mut translations = vec![[0.; 3]; model.nodes.len()];
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for &i in &model.node_order {
        translations[i] = add(
            model.nodes[i].translation,
            model.parents[i].map(|p| translations[p]).unwrap_or([0.; 3]),
        );
        for m in &model.nodes[i].meshes {
            for v in &m.vertices {
                let p = add(translations[i], v.position);
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
    }
    let root = model
        .parents
        .iter()
        .position(Option::is_none)
        .ok_or("Tree root missing")?;
    let h = hi[2] - lo[2];
    let rx = (hi[0] - lo[0]) * 0.43;
    let ry = (hi[1] - lo[1]) * 0.43;
    if !(200. ..1500.).contains(&h) {
        return Err("Unsupported tree height".into());
    }
    let mut bark = empty("OpenSC trunk and branches", BARK);
    let mut leaves = empty("OpenSC foliage", LEAVES);
    let mut random = Random(34019 + variant * 7919);
    let base = [0., 0., lo[2]];
    let crown = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        lo[2] + h * 0.79,
    ];
    let trunk = [
        base,
        [variant as f32 * 3., -8., lo[2] + h * 0.22],
        [-5., 9., lo[2] + h * 0.44],
        [crown[0] * 0.25, crown[1] * 0.25, lo[2] + h * 0.61],
    ];
    for i in 0..3 {
        tube(
            &mut bark,
            trunk[i],
            trunk[i + 1],
            h * (0.037 - i as f32 * 0.007),
            h * (0.030 - i as f32 * 0.007),
            12,
        );
    }
    // Flared roots close the trunk base and give it a grounded silhouette.
    for i in 0..6 {
        let a = i as f32 * TAU / 6. + variant as f32;
        tube(
            &mut bark,
            add(base, [a.cos() * h * 0.065, a.sin() * h * 0.065, 0.]),
            add(base, [0., 0., h * 0.10]),
            h * 0.012,
            h * 0.025,
            8,
        );
    }
    for branch in 0..14 {
        let angle = branch as f32 * TAU / 14. + variant as f32 * 0.73 + random.signed() * 0.18;
        let radial = if branch % 4 == 0 {
            0.20 + random.next() * 0.30
        } else {
            0.65 + random.next() * 0.25
        };
        let end = add(
            crown,
            [
                angle.cos() * rx * radial,
                angle.sin() * ry * radial,
                h * random.signed() * 0.075,
            ],
        );
        let start = lerp(trunk[2], trunk[3], random.next());
        let mut elbow = lerp(start, end, 0.55);
        elbow[2] -= h * 0.09;
        branch_curve(&mut bark, [start, elbow, end], [h * 0.016, h * 0.003], 9);
        for twig in 0..4 {
            let a = angle + (twig as f32 - 1.5) * 0.43;
            let tip = add(
                end,
                [
                    a.cos() * rx * 0.19,
                    a.sin() * ry * 0.19,
                    h * (0.06 + random.next() * 0.06),
                ],
            );
            tube(
                &mut bark,
                lerp(elbow, end, 0.55),
                tip,
                h * 0.004,
                h * 0.001,
                5,
            );
            for spray_id in 0..5 {
                let center = add(
                    lerp(end, tip, 0.25 + random.next() * 0.8),
                    [
                        random.signed() * rx * 0.085,
                        random.signed() * ry * 0.085,
                        random.signed() * h * 0.04,
                    ],
                );
                let size = h * (0.11 + random.next() * 0.065);
                let yaw = random.next() * TAU;
                let tilt = if spray_id % 3 == 0 {
                    1.0 + random.next() * 0.45
                } else {
                    0.15 + random.next() * 0.65
                };
                spray(
                    &mut leaves,
                    center,
                    size,
                    yaw,
                    tilt,
                    (branch + twig + spray_id) % 4,
                );
            }
        }
    }
    // Author in model space and attach to the unchanged root hierarchy.
    for mesh in [&mut bark, &mut leaves] {
        for v in &mut mesh.vertices {
            v.position = sub(v.position, translations[root]);
        }
    }
    for n in &mut model.nodes {
        n.meshes.clear();
    }
    model.nodes[root].meshes = vec![bark, leaves];
    Ok(())
}

/// Minimal SMF 1.1 authoring. Round-trip through the strict reader before any
/// file is written, so invalid normals, indices and hierarchies cannot ship.
fn encode(model: &SmfModel) -> Result<Vec<u8>, Box<dyn Error>> {
    fn text(out: &mut Vec<u8>, s: &str, n: usize) -> Result<(), Box<dyn Error>> {
        if !s.is_ascii() || s.as_bytes().contains(&0) || s.len() >= n {
            return Err("SMF authoring requires bounded ASCII identifiers".into());
        }
        out.extend_from_slice(s.as_bytes());
        out.resize(out.len() + n - s.len(), 0);
        Ok(())
    }
    fn u(out: &mut Vec<u8>, n: u32) {
        out.extend_from_slice(&n.to_le_bytes());
    }
    fn floats(out: &mut Vec<u8>, v: &[f32]) {
        for f in v {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    let mut out = vec![0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x14];
    text(&mut out, "SMF V1.1", 16)?;
    text(&mut out, &model.name, 128)?;
    floats(&mut out, &[model.radius]);
    floats(&mut out, &model.header_vector);
    u(&mut out, model.flags);
    u(&mut out, model.nodes.len() as u32);
    for n in &model.nodes {
        text(&mut out, &n.name, 128)?;
        text(&mut out, &n.parent, 128)?;
        u(&mut out, n.id);
        floats(&mut out, &n.translation);
        floats(&mut out, &n.rotation);
        u(&mut out, n.meshes.len() as u32);
        u(&mut out, n.bounds.len() as u32);
        u(&mut out, n.flags.unwrap_or(4));
        for m in &n.meshes {
            text(&mut out, &m.name, 128)?;
            text(&mut out, &m.texture, 128)?;
            u(&mut out, m.vertices.len() as u32);
            u(&mut out, m.faces.len() as u32);
            for v in &m.vertices {
                u(&mut out, v.id);
                floats(&mut out, &v.position);
                u(&mut out, v.color);
                u(&mut out, v.specular);
                floats(&mut out, &v.uv);
                floats(&mut out, &v.normal.ok_or("Generated vertex lacks normal")?);
            }
            for f in &m.faces {
                u(&mut out, f.id);
                for i in f.indices {
                    u(&mut out, i);
                }
            }
        }
        for b in &n.bounds {
            floats(&mut out, &b.coordinates);
            u(&mut out, b.tag);
        }
    }
    SmfModel::from_bytes(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel() -> SmfMesh {
        let mut m = empty("panel", "");
        for p in [
            [-100., 0., 0.],
            [100., 0., 0.],
            [100., 0., 200.],
            [-100., 0., 200.],
        ] {
            vertex(&mut m, p, [0., -1., 0.], [0.; 2]);
        }
        face(&mut m, 0, 1, 2);
        face(&mut m, 0, 2, 3);
        m
    }
    #[test]
    fn wires_union_duplicate_faces_without_filling_cut_opening() {
        let mut m = panel();
        let first = wire_intervals(&m, 1., 10.);
        m.faces.push(SmfFace {
            id: 2,
            indices: [0, 2, 1],
        });
        assert_eq!(first, wire_intervals(&m, 1., 10.));
        // Two disconnected sides must remain disconnected after clipping.
        for v in &mut m.vertices {
            v.position[0] = v.position[0].min(-30.);
        }
        let mut right = panel();
        for v in &mut right.vertices {
            v.position[0] = v.position[0].max(40.);
        }
        let offset = m.vertices.len() as u32;
        m.vertices.extend(right.vertices);
        for f in right.faces {
            m.faces.push(SmfFace {
                id: 0,
                indices: f.indices.map(|i| i + offset),
            });
        }
        assert_eq!(
            wire_intervals(&m, 0., 50.),
            vec![(-100., -30.), (40., 100.)]
        );
    }
    #[test]
    fn authored_tubes_have_outward_winding_and_finite_normals() {
        let mut m = empty("tube", STEEL);
        tube(&mut m, [0., 0., 0.], [0., 0., 100.], 5., 3., 8);
        branch_curve(
            &mut m,
            [[0., 0., 0.], [0., 0., 90.], [100., 0., 120.]],
            [5., 1.],
            9,
        );
        for v in &m.vertices {
            assert!((dot(v.normal.unwrap(), v.normal.unwrap()) - 1.).abs() < 1e-4);
        }
        for f in &m.faces {
            let [a, c, b] = f.indices.map(|i| &m.vertices[i as usize]);
            let normal = cross(sub(b.position, a.position), sub(c.position, a.position));
            assert!(
                dot(
                    normal,
                    add(add(a.normal.unwrap(), b.normal.unwrap()), c.normal.unwrap())
                ) > 0.
            );
        }
    }
}
