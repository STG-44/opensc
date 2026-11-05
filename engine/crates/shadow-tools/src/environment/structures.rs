//! Authored vehicle details and wall geometry. Installed meshes supply surface
//! anchors and attachment metadata; the engine retains the original vehicle
//! body at runtime. The private pack never changes the installation.
use super::*;

const PAINT: &str = "textures/opensc-environment/camouflage.png";
const BLOCK: &str = "textures/opensc-environment/masonry.png";
const CONCRETE: &str = "textures/opensc-environment/concrete.png";
const TRIM: &str = "textures/opensc-environment/hummer-trim.bmp";
const RUBBER: &str = "textures/opensc-environment/hummer-rubber.bmp";
const GLASS: &str = "textures/opensc-environment/hummer-glass.bmp";
const WHEELS: &str = "textures/opensc-environment/hummer-wheels.bmp";

pub(super) fn swatches(output: &Path) -> Result<(), Box<dyn Error>> {
    for (name, left, right) in [
        ("hummer-trim.bmp", [45, 48, 44], [45, 48, 44]),
        ("hummer-rubber.bmp", [27, 29, 26], [27, 29, 26]),
        ("hummer-glass.bmp", [43, 61, 65], [43, 61, 65]),
        ("hummer-wheels.bmp", [34, 32, 29], [86, 77, 62]),
        ("hummer-lamp.bmp", [220, 218, 190], [220, 218, 190]),
        ("hummer-amber.bmp", [180, 94, 22], [180, 94, 22]),
        ("hummer-red.bmp", [145, 24, 20], [145, 24, 20]),
    ] {
        // Constant color swatches supply material colors; surface detail is
        // real geometry, not painted highlights or a photographic atlas.
        let width = 16u32;
        let height = 2u32;
        let size = 54 + width * height * 3;
        let mut bmp = vec![0; size as usize];
        bmp[..2].copy_from_slice(b"BM");
        for (offset, value) in [
            (2, size),
            (10, 54),
            (14, 40),
            (18, width),
            (22, height),
            (34, size - 54),
        ] {
            bmp[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bmp[26] = 1;
        bmp[28] = 24;
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = if x < width / 2 { left } else { right };
                let p = (54 + (y * width + x) * 3) as usize;
                bmp[p..p + 3].copy_from_slice(&[b, g, r]);
            }
        }
        fs::write(output.join(name), bmp)?;
    }
    Ok(())
}

fn uv(p: V, n: V, texture: &str) -> [f32; 2] {
    if !matches!(texture, PAINT | BLOCK | CONCRETE) {
        return [0.5; 2];
    }
    let density = if texture == CONCRETE { 100. } else { 160. };
    let axes = if n[0].abs() > n[1].abs().max(n[2].abs()) {
        [1, 2]
    } else if n[1].abs() > n[2].abs() {
        [0, 2]
    } else {
        [0, 1]
    };
    [p[axes[0]] / density, -p[axes[1]] / density]
}

fn quad(m: &mut SmfMesh, mut points: [V; 4], normal: V) {
    if dot(
        cross(sub(points[1], points[0]), sub(points[2], points[0])),
        normal,
    ) < 0.
    {
        points.swap(1, 3);
    }
    let start = m.vertices.len() as u32;
    for p in points {
        vertex(m, p, normal, uv(p, normal, &m.texture));
    }
    face(m, start, start + 1, start + 2);
    face(m, start, start + 2, start + 3);
}

fn oriented_box(m: &mut SmfMesh, center: V, axes: [V; 3], half: V) {
    for axis in 0..3 {
        for sign in [-1., 1.] {
            let n = mul(axes[axis], sign);
            let c = add(center, mul(axes[axis], half[axis] * sign));
            let a = mul(axes[(axis + 1) % 3], half[(axis + 1) % 3]);
            let b = mul(axes[(axis + 2) % 3], half[(axis + 2) % 3]);
            quad(
                m,
                [
                    sub(sub(c, a), b),
                    add(sub(c, b), a),
                    add(add(c, a), b),
                    add(sub(c, a), b),
                ],
                n,
            );
        }
    }
}
fn block(m: &mut SmfMesh, low: V, high: V) {
    oriented_box(
        m,
        lerp(low, high, 0.5),
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        mul(sub(high, low), 0.5),
    );
}

/// Convex side profile extruded along X. Used for the sloping cabin, hood and
/// wall coping so every exposed end is closed and has outward normals.
fn prism(m: &mut SmfMesh, left: f32, right: f32, profile: &[[f32; 2]]) {
    for (x, n) in [(left, [-1., 0., 0.]), (right, [1., 0., 0.])] {
        for i in 1..profile.len() - 1 {
            let mut p = [profile[0], profile[i], profile[i + 1]].map(|v| [x, v[0], v[1]]);
            if dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), n) < 0. {
                p.swap(1, 2);
            }
            let ids = p.map(|p| vertex(m, p, n, uv(p, n, &m.texture)));
            face(m, ids[0], ids[1], ids[2]);
        }
    }
    let center = profile.iter().fold([0., 0.], |c, p| {
        [
            c[0] + p[0] / profile.len() as f32,
            c[1] + p[1] / profile.len() as f32,
        ]
    });
    for i in 0..profile.len() {
        let a = profile[i];
        let b = profile[(i + 1) % profile.len()];
        let mut n = unit([0., b[1] - a[1], a[0] - b[0]]);
        if dot(
            n,
            [
                0.,
                (a[0] + b[0]) * 0.5 - center[0],
                (a[1] + b[1]) * 0.5 - center[1],
            ],
        ) < 0.
        {
            n = mul(n, -1.);
        }
        quad(
            m,
            [
                [left, a[0], a[1]],
                [right, a[0], a[1]],
                [right, b[0], b[1]],
                [left, b[0], b[1]],
            ],
            n,
        );
    }
}

pub(super) fn wall(model: &mut SmfModel) -> Result<(), Box<dyn Error>> {
    let node = model
        .nodes
        .iter_mut()
        .find(|n| n.meshes.iter().any(|m| m.name == "Box02"))
        .ok_or("Wall profile missing")?;
    let source = node.meshes.first().ok_or("Wall mesh missing")?;
    if !source.texture.eq_ignore_ascii_case("con_WallComp.BMP") || source.vertices.len() < 60 {
        return Err("Unsupported masonry profile".into());
    }
    let p = |i: usize| source.vertices[i].position;
    let left = p(8)[0];
    let right = p(9)[0];
    let bottom = p(8)[2];
    let panel_top = p(10)[2];
    let cap_top = p(20)[2];
    let mut masonry = empty("Box02", BLOCK);
    let mut coping = empty("OpenSC wall coping", CONCRETE);
    // Build in model space so courses remain at a consistent ground height
    // across the three SMFs, despite their different node translations.
    let tr = node.translation;
    block(
        &mut masonry,
        add([p(1)[0], -50., bottom], tr),
        add([p(0)[0], 50., p(2)[2]], tr),
    );
    block(
        &mut masonry,
        add([left, -30., bottom], tr),
        add([right, 30., panel_top], tr),
    );
    prism(
        &mut coping,
        left + tr[0],
        right + tr[0],
        &[
            [-30. + tr[1], panel_top + tr[2]],
            [30. + tr[1], panel_top + tr[2]],
            [45. + tr[1], cap_top + tr[2]],
            [-45. + tr[1], cap_top + tr[2]],
        ],
    );
    for mesh in [&mut masonry, &mut coping] {
        for v in &mut mesh.vertices {
            v.position = sub(v.position, tr);
        }
    }
    node.meshes = vec![masonry, coping];
    Ok(())
}

fn recolor_since(m: &mut SmfMesh, start: usize, u: f32) {
    for v in &mut m.vertices[start..] {
        v.uv = [u, 0.5];
    }
}

fn wheel(source: &SmfMesh) -> SmfMesh {
    let (low, high) = bounds(source);
    let center = lerp(low, high, 0.5);
    let radius = (high[2] - low[2]) * 0.5;
    let half_width = (high[0] - low[0]) * 0.5;
    let mut m = empty(&source.name, WHEELS);
    let segments = 48u32;
    let start = m.vertices.len() as u32;
    let profile: [[f32; 2]; 8] = [
        [-1.0, 0.56],
        [-1.0, 0.77],
        [-0.90, 0.90],
        [-0.72, 0.96],
        [0.72, 0.96],
        [0.90, 0.90],
        [1.0, 0.77],
        [1.0, 0.56],
    ];
    for [x, r] in profile {
        for i in 0..=segments {
            let angle = TAU * i as f32 / segments as f32;
            let radial = [0., angle.cos(), angle.sin()];
            let normal = unit(add(
                mul(radial, if x.abs() > 0.95 { 0.25 } else { 1. }),
                [x.signum() * if x.abs() > 0.8 { 0.85 } else { 0.1 }, 0., 0.],
            ));
            vertex(
                &mut m,
                add(
                    center,
                    [
                        x * half_width,
                        radial[1] * radius * r,
                        radial[2] * radius * r,
                    ],
                ),
                normal,
                [0.25, 0.5],
            );
        }
    }
    let stride = segments + 1;
    for ring in 0..profile.len() as u32 - 1 {
        for i in 0..segments {
            let a = start + ring * stride + i;
            // Profile runs left to right; circumferential rings are clockwise
            // when viewed along +X. Orient their faces radially outwards.
            face(&mut m, a, a + 1, a + stride);
            face(&mut m, a + 1, a + stride + 1, a + stride);
        }
    }
    for i in 0..40 {
        for row in 0..3 {
            let angle = (i as f32 + if row == 1 { 0.45 } else { 0. }) * TAU / 40.;
            let radial = [0., angle.cos(), angle.sin()];
            let tangent = [0., -angle.sin(), angle.cos()];
            let tilt: f32 = if row == 0 { -0.28 } else { 0.28 };
            let u = add([tilt.cos(), 0., 0.], mul(tangent, tilt.sin()));
            let v = add([-tilt.sin(), 0., 0.], mul(tangent, tilt.cos()));
            let c = add(
                center,
                add(
                    [(row as f32 - 1.) * half_width * 0.58, 0., 0.],
                    mul(radial, radius * 0.975),
                ),
            );
            let at = m.vertices.len();
            oriented_box(
                &mut m,
                c,
                [u, v, radial],
                [half_width * 0.27, radius * 0.048, radius * 0.015],
            );
            recolor_since(&mut m, at, 0.25);
        }
    }
    // Hubs, rims and lug nuts belong to the same named wheel mesh, so steering
    // and rolling animate every detail together using the original pivot.
    for sign in [-1., 1.] {
        let at = m.vertices.len();
        let side = center[0] + sign * half_width * 1.005;
        tube(
            &mut m,
            [side, center[1], center[2]],
            [side + sign * 1.5, center[1], center[2]],
            radius * 0.55,
            radius * 0.55,
            32,
        );
        tube(
            &mut m,
            [side, center[1], center[2]],
            [side + sign * 4., center[1], center[2]],
            radius * 0.17,
            radius * 0.17,
            16,
        );
        for bolt in 0..8 {
            let angle = bolt as f32 * TAU / 8.;
            let p = [
                side,
                center[1] + angle.cos() * radius * 0.34,
                center[2] + angle.sin() * radius * 0.34,
            ];
            tube(&mut m, p, add(p, [sign * 3., 0., 0.]), 1.8, 1.8, 6);
        }
        recolor_since(&mut m, at, 0.75);
    }
    m
}

/// Intersect source faces in two dimensions and recover the outer surface.
/// Detail anchors follow the installed hood/windshield, not a generic box.
fn surface(source: &SmfMesh, tr: V, axis: usize, point: V) -> Option<f32> {
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    let mut value = f32::NEG_INFINITY;
    for face in &source.faces {
        let [a, b, c] = face
            .indices
            .map(|i| add(source.vertices[i as usize].position, tr));
        let det = (b[u] - a[u]) * (c[v] - a[v]) - (b[v] - a[v]) * (c[u] - a[u]);
        if det.abs() < 1e-6 {
            continue;
        }
        let w1 = ((point[u] - a[u]) * (c[v] - a[v]) - (point[v] - a[v]) * (c[u] - a[u])) / det;
        let w2 = ((b[u] - a[u]) * (point[v] - a[v]) - (b[v] - a[v]) * (point[u] - a[u])) / det;
        if w1 >= -1e-4 && w2 >= -1e-4 && w1 + w2 <= 1.0001 {
            value = value.max(a[axis] + w1 * (b[axis] - a[axis]) + w2 * (c[axis] - a[axis]));
        }
    }
    value.is_finite().then_some(value)
}

fn uv_surface(source: &SmfMesh, tr: V, uv: [f32; 2], front: bool) -> Option<V> {
    let mut best = None;
    for f in &source.faces {
        let v = f.indices.map(|i| &source.vertices[i as usize]);
        let [a, b, c] = v.map(|v| v.uv);
        let det = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if det.abs() < 1e-8 {
            continue;
        }
        let u = ((uv[0] - a[0]) * (c[1] - a[1]) - (uv[1] - a[1]) * (c[0] - a[0])) / det;
        let w = ((b[0] - a[0]) * (uv[1] - a[1]) - (b[1] - a[1]) * (uv[0] - a[0])) / det;
        if u >= -1e-4 && w >= -1e-4 && u + w <= 1.0001 {
            let p = add(
                tr,
                add(
                    v[0].position,
                    add(
                        mul(sub(v[1].position, v[0].position), u),
                        mul(sub(v[2].position, v[0].position), w),
                    ),
                ),
            );
            if best
                .as_ref()
                .is_none_or(|q: &V| if front { p[1] > q[1] } else { p[1] < q[1] })
            {
                best = Some(p);
            }
        }
    }
    best
}

pub(super) fn hummer(
    model: &mut SmfModel,
    optics: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut transforms = vec![[0.; 3]; model.nodes.len()];
    for &i in &model.node_order {
        transforms[i] = add(
            model.nodes[i].translation,
            model.parents[i].map(|p| transforms[p]).unwrap_or([0.; 3]),
        );
    }
    let mut tires = 0;
    for node in &mut model.nodes {
        if node.name.to_ascii_lowercase().starts_with("tire-") {
            if node.meshes.len() != 1 {
                return Err("Unsupported wheel attachment".into());
            }
            node.meshes = vec![wheel(&node.meshes[0])];
            tires += 1;
        }
    }
    if tires != 4 {
        return Err("Hummer requires four wheel attachments".into());
    }
    let chassis = model
        .nodes
        .iter()
        .position(|n| n.name == "Chasis")
        .ok_or("Hummer chassis missing")?;
    let source = model.nodes[chassis]
        .meshes
        .first()
        .ok_or("Hummer body missing")?;
    if source.name != "hummer-back01" {
        return Err("Unsupported installed Hummer body".into());
    }
    let tr = transforms[chassis];
    let mut trim = empty("OpenSC Hummer attached trim", TRIM);
    let mut rubber = empty("OpenSC Hummer hood vents", RUBBER);
    let mut glass = empty("OpenSC Hummer mirror glass", GLASS);
    let mut headlamps = empty(
        "OpenSC Hummer headlamps",
        "textures/opensc-environment/hummer-body-v1.png",
    );
    let mut tail_lamps = empty(
        "OpenSC Hummer tail lamps",
        "textures/opensc-environment/hummer-body-v1.png",
    );
    let mut wipers = Vec::new();
    let mut bindings = Vec::new();
    // The original roof taper, body contours, windows, camouflage, signs and
    // UV atlas are rendered from the installation via the version-2 sidecar.
    // No proprietary body vertices or atlas pixels are copied into this pack.
    for bar in 0..8 {
        let x = -48. + bar as f32 * 96. / 7.;
        if let (Some(a), Some(b)) = (
            surface(source, tr, 1, [x, 0., 88.]),
            surface(source, tr, 1, [x, 0., 111.]),
        ) {
            tube(
                &mut trim,
                [x, a + 0.8, 88.],
                [x, b + 0.8, 111.],
                1.1,
                1.1,
                6,
            );
        }
    }
    for optic in optics["headlamps"]
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or("Two calibrated headlamps required")?
    {
        let pair = |key: &str| -> Result<[f32; 2], Box<dyn Error>> {
            let a = optic[key]
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or("Invalid headlamp ellipse")?;
            let mut p = [0.; 2];
            for (p, v) in p.iter_mut().zip(a) {
                *p = v
                    .as_f64()
                    .filter(|v| v.is_finite() && *v > 0. && *v < 1.)
                    .ok_or("Invalid optical coordinate")? as f32;
            }
            Ok(p)
        };
        let center = pair("center_uv")?;
        let inner = pair("inner_radii_uv")?;
        let outer = pair("outer_radii_uv")?;
        if (0..2)
            .any(|i| inner[i] >= outer[i] || center[i] - outer[i] < 0. || center[i] + outer[i] > 1.)
        {
            return Err("Invalid optical ellipse bounds".into());
        }
        let sample =
            |angle: f32, radii: [f32; 2], depth: f32| -> Result<(V, [f32; 2]), Box<dyn Error>> {
                let uv = [
                    center[0] + angle.cos() * radii[0],
                    center[1] + angle.sin() * radii[1],
                ];
                let mut p =
                    uv_surface(source, tr, uv, true).ok_or("Optic outside installed front UVs")?;
                p[1] += depth;
                Ok((p, uv))
            };
        let mut center_point =
            uv_surface(source, tr, center, true).ok_or("Headlamp center outside source")?;
        center_point[1] += 0.9;
        let center_id = vertex(&mut headlamps, center_point, [0., 1., 0.], center);
        let mut rim = Vec::new();
        let mut rim_points = Vec::new();
        for i in 0..32 {
            let a = TAU * i as f32 / 32.;
            let b = TAU * (i + 1) as f32 / 32.;
            quad(
                &mut trim,
                [
                    sample(a, outer, 0.3)?.0,
                    sample(b, outer, 0.3)?.0,
                    sample(b, inner, 1.2)?.0,
                    sample(a, inner, 1.2)?.0,
                ],
                [0., 1., 0.],
            );
            let (p, uv) = sample(a, inner.map(|r| r * 0.99), 0.9)?;
            rim_points.push(p);
            rim.push(vertex(&mut headlamps, p, [0., 1., 0.], uv));
        }
        for i in 0..32 {
            let next = (i + 1) % 32;
            if dot(
                cross(
                    sub(rim_points[i], center_point),
                    sub(rim_points[next], center_point),
                ),
                [0., 1., 0.],
            ) > 0.
            {
                face(&mut headlamps, center_id, rim[i], rim[next]);
            } else {
                face(&mut headlamps, center_id, rim[next], rim[i]);
            }
        }
    }
    for sign in [-1., 1.] {
        let name = if sign < 0. {
            "OpenSC Hummer wiper left"
        } else {
            "OpenSC Hummer wiper right"
        };
        let project = |x: f32, z: f32| -> Result<V, Box<dyn Error>> {
            let y = surface(source, tr, 1, [x, 0., z]).ok_or("Wiper anchor missing")?;
            Ok([x, y + 1.2, z])
        };
        // Wiper motors are mounted on the upper windshield rail.
        let pivot = project(sign * 55., 200.5)?;
        // Sample inside the pane: the roof above the rail is a different face.
        let y_below = project(sign * 55., 175.)?[1];
        let y_above = project(sign * 55., 185.)?[1];
        let axis = unit([0., 1., (y_below - y_above) / 10.]);
        let mut wiper = empty(name, RUBBER);
        tube(&mut wiper, pivot, project(sign * 75., 174.)?, 0.65, 0.65, 6);
        tube(
            &mut wiper,
            project(sign * 75., 157.)?,
            project(sign * 75., 190.)?,
            1.,
            1.,
            6,
        );
        tube(&mut wiper, sub(pivot, axis), add(pivot, axis), 2.2, 2.2, 8);
        bindings.push(serde_json::json!({"part":name,"pivot":pivot,"axis":axis,"sweep_radians":sign*70f32.to_radians()}));
        wipers.push(wiper);
        let x = sign * 115.;
        if let Some(y) = surface(source, tr, 1, [x, 0., 159.]) {
            tube(
                &mut trim,
                [x, y + 0.5, 159.],
                [sign * 126., y - 3., 164.],
                1.,
                1.,
                6,
            );
            block(
                &mut trim,
                [sign * 128. - 1.7, y - 7., 158.],
                [sign * 128. + 1.7, y + 4., 175.],
            );
            quad(
                &mut glass,
                [
                    [sign * 129.9, y - 6., 159.],
                    [sign * 129.9, y + 3., 159.],
                    [sign * 129.9, y + 3., 174.],
                    [sign * 129.9, y - 6., 174.],
                ],
                [sign, 0., 0.],
            );
        }
    }
    for (left, right) in [(0.489, 0.511), (0.769, 0.791)] {
        let mut coords = [[left, 0.067], [right, 0.067], [right, 0.099], [left, 0.099]];
        let mut points = coords
            .map(|uv| uv_surface(source, tr, uv, false).ok_or("Tail lamp atlas mapping missing"))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        if dot(
            cross(sub(points[1], points[0]), sub(points[2], points[0])),
            [0., -1., 0.],
        ) < 0.
        {
            points.swap(1, 3);
            coords.swap(1, 3);
        }
        let start = tail_lamps.vertices.len() as u32;
        for (mut p, uv) in points.into_iter().zip(coords) {
            p[1] -= 0.8;
            vertex(&mut tail_lamps, p, [0., -1., 0.], uv);
        }
        face(&mut tail_lamps, start, start + 1, start + 2);
        face(&mut tail_lamps, start, start + 2, start + 3);
    }
    for row in 0..6 {
        let y = 316. + row as f32 * 4.;
        if let Some(z) = surface(source, tr, 2, [0., y, 0.]) {
            block(&mut rubber, [-36., y, z + 0.2], [36., y + 1., z + 0.8]);
        }
    }
    let mut meshes = vec![trim, rubber, glass, headlamps, tail_lamps];
    meshes.extend(wipers);
    for m in &mut meshes {
        for v in &mut m.vertices {
            v.position = sub(v.position, tr);
        }
    }
    model.nodes[chassis].meshes = meshes;
    Ok(serde_json::json!(bindings))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optical_uv_projection_preserves_anisotropic_texture_registration() {
        let mut source = empty("test", "");
        for (p, uv) in [
            ([0., 10., 0.], [0., 0.]),
            ([100., 10., 0.], [1., 0.]),
            ([0., 10., 200.], [0., 1.]),
        ] {
            vertex(&mut source, p, [0., 1., 0.], uv);
        }
        face(&mut source, 0, 2, 1);
        let p = uv_surface(&source, [0.; 3], [0.25, 0.3], true).unwrap();
        assert!((p[0] - 25.).abs() < 1e-5);
        assert!((p[2] - 60.).abs() < 1e-4);
        assert_eq!(p[1], 10.);
        assert!(uv_surface(&source, [0.; 3], [0.9, 0.9], true).is_none());
    }
    #[test]
    fn box_and_closed_prism_faces_point_outward() {
        let mut m = empty("test", CONCRETE);
        block(&mut m, [-1., -2., -3.], [1., 2., 3.]);
        prism(&mut m, -5., 5., &[[-1., 0.], [1., 0.], [2., 3.], [-2., 3.]]);
        for f in &m.faces {
            let [a, c, b] = f.indices.map(|i| &m.vertices[i as usize]);
            let normal = cross(sub(b.position, a.position), sub(c.position, a.position));
            assert!(dot(normal, a.normal.unwrap()) > 0.);
        }
    }
    #[test]
    fn wheel_keeps_original_center_and_one_animated_mesh_name() {
        let mut source = empty("tr2", "");
        vertex(&mut source, [-20., -50., -50.], [1., 0., 0.], [0.; 2]);
        vertex(&mut source, [20., 50., 50.], [1., 0., 0.], [0.; 2]);
        let m = wheel(&source);
        assert_eq!(m.name, "tr2");
        let (a, b) = bounds(&m);
        let center = lerp(a, b, 0.5);
        assert!(center.into_iter().all(|x| x.abs() < 0.02));
        assert!((b[2] - 50.).abs() < 1.);
        for f in &m.faces {
            let [a, c, b] = f.indices.map(|i| &m.vertices[i as usize]);
            let n = cross(sub(b.position, a.position), sub(c.position, a.position));
            assert!(
                dot(
                    n,
                    add(add(a.normal.unwrap(), b.normal.unwrap()), c.normal.unwrap())
                ) > 0.
            );
        }
        assert!(
            m.vertices
                .iter()
                .all(|v| v.normal.unwrap().into_iter().all(f32::is_finite))
        );
    }
}
