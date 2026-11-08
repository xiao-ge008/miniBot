//! 二维码 / 条码：识别（rxing）+ 生成（qrcode → SVG）
use image::RgbaImage;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct QrResult {
    pub text: String,
    pub format: String,
    /// 角点（图片像素坐标）
    pub points: Vec<(f32, f32)>,
}

fn decode_luma(luma: Vec<u8>, w: u32, h: u32) -> Vec<QrResult> {
    match rxing::helpers::detect_multiple_in_luma(luma, w, h) {
        Ok(list) => list
            .into_iter()
            .map(|r| QrResult {
                text: r.getText().to_string(),
                format: format!("{:?}", r.getBarcodeFormat()),
                points: r.getPoints().iter().map(|p| (p.x, p.y)).collect(),
            })
            .collect(),
        Err(_) => vec![],
    }
}

/// 识别图片中的所有二维码/条码；识别失败时放大 2 倍再试一次（提升小码识别率）
pub fn decode_rgba(img: &RgbaImage) -> Vec<QrResult> {
    let gray = image::DynamicImage::ImageRgba8(img.clone()).to_luma8();
    let (w, h) = gray.dimensions();
    let res = decode_luma(gray.clone().into_raw(), w, h);
    if !res.is_empty() || w > 2000 || h > 2000 {
        return res;
    }
    let big = image::imageops::resize(&gray, w * 2, h * 2, image::imageops::FilterType::Triangle);
    decode_luma(big.into_raw(), w * 2, h * 2)
        .into_iter()
        .map(|mut r| {
            r.points.iter_mut().for_each(|p| {
                p.0 /= 2.0;
                p.1 /= 2.0;
            });
            r
        })
        .collect()
}

/// 生成二维码 SVG。ec: L/M/Q/H
pub fn encode_svg(text: &str, ec: &str, dark: &str, light: &str) -> anyhow::Result<String> {
    use qrcode::{render::svg, EcLevel, QrCode};
    let level = match ec {
        "L" => EcLevel::L,
        "Q" => EcLevel::Q,
        "H" => EcLevel::H,
        _ => EcLevel::M,
    };
    let code = QrCode::with_error_correction_level(text.as_bytes(), level)?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .dark_color(svg::Color(dark))
        .light_color(svg::Color(light))
        .build())
}
