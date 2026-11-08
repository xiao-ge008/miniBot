//! 系统 OCR：Windows.Media.Ocr（零体积、毫秒级）
use image::RgbaImage;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct OcrLine {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrResult {
    pub text: String,
    pub lines: Vec<OcrLine>,
    pub lang: String,
}

#[allow(dead_code)]
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x303F | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0xAC00..=0xD7AF)
}

/// Windows OCR 会在中日韩字符之间插入空格，这里去掉
#[allow(dead_code)]
fn tidy_cjk(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' && i > 0 && i + 1 < chars.len() && (is_cjk(chars[i - 1]) || is_cjk(chars[i + 1])) {
            // 两侧至少一侧为 CJK 且另一侧不是拉丁单词的连续部分
            if is_cjk(chars[i - 1]) && is_cjk(chars[i + 1]) || is_cjk(chars[i - 1]) && chars[i + 1].is_ascii_punctuation() || is_cjk(chars[i + 1]) && chars[i - 1].is_ascii_punctuation() {
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(windows)]
pub fn recognize(img: &RgbaImage) -> anyhow::Result<OcrResult> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }

    // 太小的图放大，提升识别率；太大则缩小到引擎上限内
    let max = OcrEngine::MaxImageDimension()? as u32;
    let (mut w, mut h) = img.dimensions();
    let mut scale = 1.0f32;
    let mut work = img.clone();
    if h < 60 {
        scale = (60.0 / h as f32).min(4.0);
    }
    if (w as f32 * scale) as u32 > max || (h as f32 * scale) as u32 > max {
        scale = max as f32 / w.max(h) as f32;
    }
    if (scale - 1.0).abs() > 0.01 {
        w = ((w as f32) * scale) as u32;
        h = ((h as f32) * scale) as u32;
        work = image::imageops::resize(img, w, h, image::imageops::FilterType::CatmullRom);
    }

    // RGBA → BGRA
    let mut bgra = work.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w as i32, h as i32)?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
    let lang = engine.RecognizerLanguage().and_then(|l| l.LanguageTag()).map(|s| s.to_string()).unwrap_or_default();
    let result = engine.RecognizeAsync(&bitmap)?.get()?;

    let mut lines = vec![];
    for line in result.Lines()? {
        let text = tidy_cjk(&line.Text()?.to_string());
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, 0f32, 0f32);
        for word in line.Words()? {
            let r = word.BoundingRect()?;
            x0 = x0.min(r.X);
            y0 = y0.min(r.Y);
            x1 = x1.max(r.X + r.Width);
            y1 = y1.max(r.Y + r.Height);
        }
        lines.push(OcrLine { text, x: x0 / scale, y: y0 / scale, w: (x1 - x0) / scale, h: (y1 - y0) / scale });
    }
    let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    Ok(OcrResult { text, lines, lang })
}

#[cfg(not(windows))]
pub fn recognize(_img: &RgbaImage) -> anyhow::Result<OcrResult> {
    Ok(OcrResult {
        text: String::new(),
        lines: vec![],
        lang: "zh-Hans".into(),
    })
}
