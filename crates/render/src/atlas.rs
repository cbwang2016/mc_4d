//! Original Beta atlases and skins. Source/license attribution is kept beside
//! the unchanged assets; these images are not covered by the Rust code license.
use std::{io::Cursor, path::Path};

pub const SIDE: u32 = 256;
pub const LAYERS: u32 = 39;
/// First of ten armour layers: cloth, chain, iron, diamond, gold (item id order),
/// each as `_1` (helmet/chest/boots) then `_2` (leggings).
pub const ARMOR_LAYER: u32 = 29;
pub const PATHS: [&str; LAYERS as usize] = [
    "terrain.png", "gui/items.png", "mob/pig.png", "mob/cow.png", "mob/sheep.png",
    "mob/chicken.png", "mob/squid.png", "mob/zombie.png", "mob/skeleton.png",
    "mob/spider.png", "mob/creeper.png", "mob/slime.png", "mob/wolf.png",
    "mob/wolf_tame.png", "mob/wolf_angry.png", "mob/sheep_fur.png",
    "environment/rain.png", "environment/snow.png", "item/sign.png",
    "misc/grasscolor.png", "misc/foliagecolor.png", "mob/char.png", "item/boat.png", "item/cart.png", "terrain/sun.png", "terrain/moon.png", "environment/clouds.png", "art/kz.png", "particles.png",
    "armor/cloth_1.png", "armor/cloth_2.png", "armor/chain_1.png", "armor/chain_2.png", "armor/iron_1.png", "armor/iron_2.png",
    "armor/diamond_1.png", "armor/diamond_2.png", "armor/gold_1.png", "armor/gold_2.png",
];
const BUNDLED: [&[u8]; LAYERS as usize] = [
    include_bytes!("../assets/minecraft-beta-1.5/terrain.png"),
    include_bytes!("../assets/minecraft-beta-1.5/gui/items.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/pig.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/cow.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/sheep.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/chicken.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/squid.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/zombie.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/skeleton.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/spider.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/creeper.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/slime.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/wolf.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/wolf_tame.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/wolf_angry.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/sheep_fur.png"),
    include_bytes!("../assets/minecraft-beta-1.5/environment/rain.png"),
    include_bytes!("../assets/minecraft-beta-1.5/environment/snow.png"),
    include_bytes!("../assets/minecraft-beta-1.5/item/sign.png"),
    include_bytes!("../assets/minecraft-beta-1.5/misc/grasscolor.png"),
    include_bytes!("../assets/minecraft-beta-1.5/misc/foliagecolor.png"),
    include_bytes!("../assets/minecraft-beta-1.5/mob/char.png"),
    include_bytes!("../assets/minecraft-beta-1.5/item/boat.png"),
    include_bytes!("../assets/minecraft-beta-1.5/item/cart.png"),
    include_bytes!("../assets/minecraft-beta-1.5/terrain/sun.png"),
    include_bytes!("../assets/minecraft-beta-1.5/terrain/moon.png"),
    include_bytes!("../assets/minecraft-beta-1.5/environment/clouds.png"),
    include_bytes!("../assets/minecraft-beta-1.5/art/kz.png"),
    include_bytes!("../assets/minecraft-beta-1.5/particles.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/cloth_1.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/cloth_2.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/chain_1.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/chain_2.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/iron_1.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/iron_2.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/diamond_1.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/diamond_2.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/gold_1.png"),
    include_bytes!("../assets/minecraft-beta-1.5/armor/gold_2.png"),
];

pub fn load(folder: Option<&Path>) -> Result<Vec<u8>, String> {
    if let Some(pack) = folder { crate::pack::validate(pack)?; }
    let layer_bytes = (SIDE * SIDE * 4) as usize;
    let mut pixels = vec![0; layer_bytes * LAYERS as usize];
    for (layer, name) in PATHS.iter().enumerate() {
        let external = folder.and_then(|p| crate::pack::read(p, name));
        let data = external.as_deref().unwrap_or(BUNDLED[layer]);
        let (width, height, rgba) = decode(data).map_err(|e| format!("{name}: {e}"))?;
        let (target_w, target_h) = if layer < 2 || layer == 19 || layer == 20 || layer == 26 || layer == 27 { (256, 256) } else if layer == 28 { (128, 128) } else if layer == 24 || layer == 25 { (32, 32) } else if layer == 16 || layer == 17 { (64, 256) } else { (64, 32) };
        if width * target_h != height * target_w { return Err(format!("{name}: unexpected aspect ratio {width}x{height}")); }
        // Texture packs can supply larger exact-ratio images. Nearest sampling
        // retains the original pixel grid and never creates blurred seams.
        for y in 0..target_h { for x in 0..target_w {
            let src = (((y * height / target_h) * width + x * width / target_w) * 4) as usize;
            let dst = layer * layer_bytes + ((y * SIDE + x) * 4) as usize;
            pixels[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        } }
    }
    Ok(pixels)
}

pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 || info.width > 4096 || info.height > 4096 { return Err("image dimensions must be 1..4096".into()); }
    let mut raw = vec![0; reader.output_buffer_size().ok_or("PNG buffer too large")?];
    let frame = reader.next_frame(&mut raw).map_err(|e| e.to_string())?;
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4, png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2, png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err("unexpanded indexed PNG".into()),
    };
    let mut rgba = Vec::with_capacity((frame.width * frame.height * 4) as usize);
    for p in raw[..frame.buffer_size()].chunks_exact(channels) {
        match channels {
            4 => rgba.extend_from_slice(p),
            3 => rgba.extend_from_slice(&[p[0], p[1], p[2], 255]),
            2 => rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
            _ => rgba.extend_from_slice(&[p[0], p[0], p[0], 255]),
        }
    }
    Ok((frame.width, frame.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_assets_decode_without_changing_the_terrain_pixels() {
        let layers = load(None).unwrap();
        assert_eq!(layers.len(), (SIDE * SIDE * 4 * LAYERS) as usize);
        let (_, _, original) = decode(BUNDLED[0]).unwrap();
        assert_eq!(&layers[..original.len()], &original);
        for layer in 0..LAYERS as usize {
            assert!(layers[layer * 256 * 256 * 4..(layer + 1) * 256 * 256 * 4].iter().any(|&v| v != 0));
        }
    }
}
