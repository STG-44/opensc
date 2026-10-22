use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, MtfMap, PcxImage};
use std::error::Error;
pub fn describe(catalog: &AssetCatalog, path: &str) -> Result<Value, Box<dyn Error>> {
    let map = MtfMap::from_bytes(&catalog.read(path)?)?;
    Ok(
        json!({"path":shadow_formats::normalize_path(path)?, "globals":map.globals.iter().map(|l| &l.words).collect::<Vec<_>>(), "objects":map.objects.iter().map(|o| json!({"inventory":o.inventory,"class":o.class,"model":o.model,"name":o.name,"position":o.position,"rotation":o.rotation,"id":o.id,"properties":o.properties.iter().map(|l|&l.words).collect::<Vec<_>>() })).collect::<Vec<_>>() }),
    )
}
pub fn heightmap(catalog: &AssetCatalog, path: &str) -> Result<Value, Box<dyn Error>> {
    let image = PcxImage::from_bytes(&catalog.read(path)?)?;
    Ok(
        json!({"path":shadow_formats::normalize_path(path)?,"width":image.width,"height":image.height,"min_index":image.pixels.iter().min(),"max_index":image.pixels.iter().max(),"pixels_sha256":format!("{:x}",Sha256::digest(&image.pixels))}),
    )
}
