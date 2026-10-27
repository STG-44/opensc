use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, BmfMotion};
use std::error::Error;

pub fn describe(catalog: &AssetCatalog, path: &str) -> Result<Value, Box<dyn Error>> {
    let motion = BmfMotion::from_bytes(&catalog.read(path)?)?;
    let mut digest = Sha256::new();
    let mut key_count = 0;
    for frame in &motion.frames {
        for value in frame.root_translation {
            digest.update(value.to_le_bytes());
        }
        digest.update(frame.index.to_le_bytes());
        for key in &frame.keys {
            key_count += 1;
            digest.update(frame.index.to_le_bytes());
            digest.update(key.bone_id.to_le_bytes());
            let mask =
                u8::from(key.rotation_wxyz.is_some()) | (u8::from(key.translation.is_some()) << 1);
            digest.update([mask]);
            if let Some(values) = key.rotation_wxyz {
                for value in values {
                    digest.update(value.to_le_bytes());
                }
            }
            if let Some(values) = key.translation {
                for value in values {
                    digest.update(value.to_le_bytes());
                }
            }
        }
    }
    Ok(
        json!({"path":path, "name":motion.name, "frame_count":motion.frames.len(),
        "key_count":key_count,"frames_per_second":motion.frames_per_second,
        "ticks_per_frame":motion.ticks_per_frame,"stored_frame_millis":motion.stored_frame_millis,
        "flags":motion.flags,"bone_ids":motion.bone_ids,"duration_seconds":motion.duration_seconds(),
        "canonical_sha256":format!("{:x}",digest.finalize())}),
    )
}
