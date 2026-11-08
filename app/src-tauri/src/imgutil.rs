//! 图片编解码小工具
use base64::Engine;
use image::{ImageEncoder, RgbaImage};

/// 快速 PNG 编码（截图场景优先速度）
pub fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::with_capacity((img.width() * img.height()) as usize);
    let enc = image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    let _ = enc.write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8);
    out
}

/// 解析 data URI 或纯 base64 为字节
pub fn decode_b64(s: &str) -> anyhow::Result<Vec<u8>> {
    let raw = match s.find("base64,") {
        Some(i) => &s[i + 7..],
        None => s,
    };
    Ok(base64::engine::general_purpose::STANDARD.decode(raw.trim())?)
}

pub fn decode_rgba(bytes: &[u8]) -> anyhow::Result<RgbaImage> {
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

pub fn to_data_uri(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}
