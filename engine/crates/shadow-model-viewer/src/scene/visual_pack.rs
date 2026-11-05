//! A visual-only pack must match both its installed interaction source and SMF.
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, AssetOrigin};
use std::collections::BTreeMap;
use std::error::Error;

pub(super) struct TextureRule {
    pub texture: String,
    pub glass_regions: Vec<[f32; 4]>,
}

pub(super) struct BaseVisual {
    pub bytes: Vec<u8>,
    pub render_parts: Vec<String>,
    pub textures: BTreeMap<String, TextureRule>,
    pub wipers: Vec<super::WiperBinding>,
}

pub(super) fn base_bytes(
    catalog: &AssetCatalog,
    path: &str,
    visual: &[u8],
) -> Result<Option<BaseVisual>, Box<dyn Error>> {
    let metadata = format!("{path}.visual.json");
    if !matches!(catalog.origin(path), Ok(AssetOrigin::Override(_)))
        || !matches!(catalog.origin(&metadata), Ok(AssetOrigin::Override(_)))
    {
        return Ok(None);
    }
    let bytes = catalog.read(&metadata)?;
    if bytes.len() > 4096 {
        return Err("Visual pack metadata exceeds 4 KiB".into());
    }
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)?;
    let base = catalog.read_base(path)?;
    verify(&manifest, &base, visual)?;
    let render_parts = retained_parts(&manifest)?;
    let textures = texture_rules(&manifest)?;
    let wipers = wiper_bindings(&manifest)?;
    for rule in textures.values() {
        if !matches!(catalog.origin(&rule.texture), Ok(AssetOrigin::Override(_))) {
            return Err(format!(
                "Visual pack texture is missing from overrides: {}",
                rule.texture
            )
            .into());
        }
    }
    Ok(Some(BaseVisual {
        bytes: base,
        render_parts,
        textures,
        wipers,
    }))
}
fn wiper_bindings(m: &serde_json::Value) -> Result<Vec<super::WiperBinding>, Box<dyn Error>> {
    use bevy::prelude::*;
    let Some(value) = m.get("wipers") else {
        return Ok(Vec::new());
    };
    if m["version"].as_u64() != Some(2) {
        return Err("Wipers require visual pack version 2".into());
    }
    let list = value
        .as_array()
        .filter(|v| v.len() <= 8)
        .ok_or("Invalid wiper bindings")?;
    let vector = |v: &serde_json::Value| -> Result<Vec3, Box<dyn Error>> {
        let a = v
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or("Invalid wiper vector")?;
        let mut p = [0.; 3];
        for (p, v) in p.iter_mut().zip(a) {
            *p = v
                .as_f64()
                .filter(|v| v.is_finite() && v.abs() < 100000.)
                .ok_or("Invalid wiper coordinate")? as f32;
        }
        Ok(Vec3::from_array(p))
    };
    let basis = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let mut result = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for v in list {
        let part = v["part"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() < 128)
            .ok_or("Invalid wiper part")?
            .to_owned();
        if !names.insert(part.clone()) {
            return Err("Duplicate wiper part".into());
        }
        let pivot = basis * vector(&v["pivot"])?;
        let axis = (basis * vector(&v["axis"])?)
            .try_normalize()
            .ok_or("Zero wiper axis")?;
        let sweep = v["sweep_radians"]
            .as_f64()
            .filter(|v| v.is_finite() && v.abs() <= 1.6)
            .ok_or("Invalid wiper sweep")? as f32;
        result.push(super::WiperBinding {
            part,
            pivot,
            axis,
            sweep,
        });
    }
    Ok(result)
}

fn texture_rules(
    manifest: &serde_json::Value,
) -> Result<BTreeMap<String, TextureRule>, Box<dyn Error>> {
    let Some(value) = manifest.get("texture_overrides") else {
        return Ok(BTreeMap::new());
    };
    if manifest["version"].as_u64() != Some(2) {
        return Err("Texture remapping requires visual pack version 2".into());
    }
    let rules = value
        .as_object()
        .filter(|m| m.len() <= 16)
        .ok_or("Invalid visual pack texture remapping")?;
    let mut result = BTreeMap::new();
    for (source, value) in rules {
        let source = shadow_formats::normalize_path(source)?;
        let target = shadow_formats::normalize_path(
            value["texture"]
                .as_str()
                .ok_or("Missing replacement texture")?,
        )?;
        if !source.starts_with("textures/")
            || !target.starts_with("textures/")
            || !target.ends_with(".png")
        {
            return Err(
                "Visual texture remapping requires a texture path and PNG replacement".into(),
            );
        }
        let mut regions = Vec::new();
        if let Some(value) = value.get("glass_regions") {
            for value in value
                .as_array()
                .filter(|r| r.len() <= 16)
                .ok_or("Invalid glass regions")?
            {
                let list = value
                    .as_array()
                    .filter(|r| r.len() == 4)
                    .ok_or("Invalid glass rectangle")?;
                let mut rect = [0.; 4];
                for (out, v) in rect.iter_mut().zip(list) {
                    *out = v
                        .as_f64()
                        .filter(|v| (0.0..=1.0).contains(v))
                        .ok_or("Invalid glass coordinate")? as f32;
                }
                if rect[0] >= rect[2] || rect[1] >= rect[3] {
                    return Err("Empty glass rectangle".into());
                }
                regions.push(rect);
            }
        }
        if result
            .insert(
                source,
                TextureRule {
                    texture: target,
                    glass_regions: regions,
                },
            )
            .is_some()
        {
            return Err("Duplicate visual texture remapping".into());
        }
    }
    Ok(result)
}
fn retained_parts(manifest: &serde_json::Value) -> Result<Vec<String>, Box<dyn Error>> {
    if manifest["version"].as_u64() == Some(2) {
        let names = manifest["render_base_parts"]
            .as_array()
            .ok_or("Missing retained model parts")?;
        if names.is_empty() || names.len() > 32 {
            return Err("Invalid retained model part count".into());
        }
        let parts = names
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty() && s.len() < 128)
                    .map(str::to_owned)
                    .ok_or_else(|| "Invalid retained model part name".into())
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        if parts
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != parts.len()
        {
            return Err("Duplicate retained model part".into());
        }
        Ok(parts)
    } else {
        Ok(Vec::new())
    }
}

fn verify(manifest: &serde_json::Value, base: &[u8], visual: &[u8]) -> Result<(), Box<dyn Error>> {
    if !matches!(manifest["version"].as_u64(), Some(1 | 2))
        || manifest["source_sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(base)))
        || manifest["visual_sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(visual)))
    {
        return Err(
            "Visual pack does not match its source or replacement; rebuild the pack".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_cannot_be_reused_with_another_original_or_visual() {
        let manifest = serde_json::json!({"version":1,
            "source_sha256":format!("{:x}",Sha256::digest(b"installed")),
            "visual_sha256":format!("{:x}",Sha256::digest(b"visual"))});
        assert!(verify(&manifest, b"installed", b"visual").is_ok());
        assert!(verify(&manifest, b"other", b"visual").is_err());
        assert!(verify(&manifest, b"installed", b"custom visual").is_err());
    }
    #[test]
    fn retained_body_parts_require_explicit_v2_and_unique_valid_names() {
        let mut m = serde_json::json!({"version":2,"render_base_parts":["hummer-back01"]});
        assert_eq!(retained_parts(&m).unwrap(), vec!["hummer-back01"]);
        m["render_base_parts"] = serde_json::json!(["body", "body"]);
        assert!(retained_parts(&m).is_err());
        m["render_base_parts"] = serde_json::json!([]);
        assert!(retained_parts(&m).is_err());
        m["version"] = serde_json::json!(1);
        assert!(retained_parts(&m).unwrap().is_empty());
    }
    #[test]
    fn texture_rules_validate_paths_versions_and_glass_bounds() {
        let mut m = serde_json::json!({"version":2,"texture_overrides":{"textures/shared/hummer.bmp":{
            "texture":"textures/opensc-environment/hummer-body-v1.png","glass_regions":[[0.2,0.3,0.4,0.5]]}}});
        assert_eq!(
            texture_rules(&m).unwrap()["textures/shared/hummer.bmp"]
                .glass_regions
                .len(),
            1
        );
        m["version"] = serde_json::json!(1);
        assert!(texture_rules(&m).is_err());
        m["version"] = serde_json::json!(2);
        let rule = &mut m["texture_overrides"]["textures/shared/hummer.bmp"];
        rule["texture"] = serde_json::json!("../original.png");
        assert!(texture_rules(&m).is_err());
        m["texture_overrides"]["textures/shared/hummer.bmp"]["texture"] =
            serde_json::json!("textures/new.png");
        m["texture_overrides"]["textures/shared/hummer.bmp"]["glass_regions"] =
            serde_json::json!([[0.4, 0.3, 0.2, 1.1]]);
        assert!(texture_rules(&m).is_err());
        assert!(
            texture_rules(&serde_json::json!({"version":1}))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn wipers_convert_source_axes_and_reject_bad_or_duplicate_bindings() {
        let mut m = serde_json::json!({"version":2,"wipers":[{"part":"wiper","pivot":[10.,20.,30.],"axis":[0.,1.,0.],"sweep_radians":0.5}]});
        let a = wiper_bindings(&m).unwrap();
        assert!((a[0].pivot - bevy::prelude::Vec3::new(10., 30., -20.)).length() < 1e-4);
        m["wipers"][0]["axis"] = serde_json::json!([0., 0., 0.]);
        assert!(wiper_bindings(&m).is_err());
    }
}
