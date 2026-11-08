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

#[cfg(target_os = "macos")]
pub fn recognize(img: &RgbaImage) -> anyhow::Result<OcrResult> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static OCR_SEQ: AtomicU64 = AtomicU64::new(1);

    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(OcrResult {
            text: String::new(),
            lines: vec![],
            lang: "zh-Hans".into(),
        });
    }

    let seq = OCR_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = std::env::temp_dir().join(format!("minibot_ocr_{}_{}.png", std::process::id(), seq));
    let png_bytes = crate::imgutil::encode_png(img);
    std::fs::write(&tmp_path, png_bytes)?;

    // 调用 macOS 系统内置 JXA Objective-C Bridge 驱动 Apple Vision Framework (VNRecognizeTextRequest)
    let jxa_script = r#"
ObjC.import('Foundation');
ObjC.import('Vision');

function run(argv) {
    var imgPath = argv[0];
    var url = $.NSURL.fileURLWithPath(imgPath);
    var req = $.VNRecognizeTextRequest.alloc.init;
    req.recognitionLevel = 0; // VNRequestTextRecognitionLevelAccurate
    req.usesLanguageCorrection = true;
    try {
        req.recognitionLanguages = $(["zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR"]);
    } catch (e) {}
    var handler = $.VNImageRequestHandler.alloc.initWithURLOptions(url, $({}));
    var err = Ref();
    var ok = handler.performRequestsError($([req]), err);
    if (!ok) {
        return "[]";
    }
    var results = req.results;
    var out = [];
    if (results) {
        var count = results.count;
        for (var i = 0; i < count; i++) {
            var obs = results.objectAtIndex(i);
            var cand = obs.topCandidates(1);
            if (cand && cand.count > 0) {
                var str = ObjC.unwrap(cand.objectAtIndex(0).string);
                var box = obs.boundingBox;
                out.push({
                    text: str,
                    nx: Number(box.origin.x),
                    ny: Number(1.0 - box.origin.y - box.size.height),
                    nw: Number(box.size.width),
                    nh: Number(box.size.height)
                });
            }
        }
    }
    return JSON.stringify(out);
}
"#;

    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", jxa_script, &tmp_path.to_string_lossy()])
        .output();
    let _ = std::fs::remove_file(&tmp_path);

    let out = out?;
    if !out.status.success() {
        return Ok(OcrResult {
            text: String::new(),
            lines: vec![],
            lang: "zh-Hans".into(),
        });
    }

    #[derive(serde::Deserialize)]
    struct RawBox {
        text: String,
        nx: f32,
        ny: f32,
        nw: f32,
        nh: f32,
    }

    let json_str = String::from_utf8_lossy(&out.stdout);
    let raw_list: Vec<RawBox> = serde_json::from_str(json_str.trim()).unwrap_or_default();
    let wf = w as f32;
    let hf = h as f32;

    let mut lines: Vec<OcrLine> = raw_list
        .into_iter()
        .filter_map(|item| {
            let t = tidy_cjk(item.text.trim());
            if t.is_empty() {
                return None;
            }
            Some(OcrLine {
                text: t,
                x: (item.nx * wf).max(0.0),
                y: (item.ny * hf).max(0.0),
                w: (item.nw * wf).max(1.0),
                h: (item.nh * hf).max(1.0),
            })
        })
        .collect();

    // 按从上到下、同行从左到右排序
    lines.sort_by(|a, b| {
        let row_tol = (a.h.min(b.h) * 0.5).max(6.0);
        if (a.y - b.y).abs() <= row_tol {
            a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    Ok(OcrResult {
        text,
        lines,
        lang: "zh-Hans".into(),
    })
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn recognize(_img: &RgbaImage) -> anyhow::Result<OcrResult> {
    Ok(OcrResult {
        text: String::new(),
        lines: vec![],
        lang: "zh-Hans".into(),
    })
}
