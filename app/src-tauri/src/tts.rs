//! Edge TTS 在线神经网络语音合成模块（基于 https://github.com/rany2/edge-tts 协议规范）
//! 支持 Sec-MS-GEC v2 (CS64 + Custom SHA-256) 动态令牌生成、时钟偏差自动校准、WebSocket MP3 流式合成与内存 LRU 缓存

use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

const HOST: &str = "speech.platform.bing.com";
const PORT: u16 = 443;
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_MAJOR_VERSION: &str = "143";
const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";
const WIN_EPOCH: i64 = 11_644_473_600;

/// 时钟偏差校准值（秒）：当本机系统时间与微软服务器 UTC 时间不一致触发 403 时，自动根据响应头 Date 校准
static CLOCK_SKEW_SECS: AtomicI64 = AtomicI64::new(0);
static REQ_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 内存音频缓存（最多缓存 32 段合成好的 data:audio/mp3;base64,...）
static AUDIO_CACHE: OnceLock<Mutex<VecDeque<(String, String)>>> = OnceLock::new();

fn cache_get(key: &str) -> Option<String> {
    let lock = AUDIO_CACHE.get_or_init(|| Mutex::new(VecDeque::with_capacity(32)));
    let guard = lock.lock().ok()?;
    guard.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn cache_put(key: String, val: String) {
    let lock = AUDIO_CACHE.get_or_init(|| Mutex::new(VecDeque::with_capacity(32)));
    if let Ok(mut guard) = lock.lock() {
        if let Some(pos) = guard.iter().position(|(k, _)| k == &key) {
            guard.remove(pos);
        }
        if guard.len() >= 32 {
            guard.pop_front();
        }
        guard.push_back((key, val));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceOption {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub gender: String,
    pub desc: String,
}

/// 内置精选高自然度 Edge TTS Neural 语音列表
pub fn builtin_voices() -> Vec<VoiceOption> {
    vec![
        VoiceOption {
            id: "zh-CN-XiaoxiaoNeural".into(),
            name: "晓晓 (Xiaoxiao)".into(),
            lang: "zh-CN".into(),
            gender: "女声".into(),
            desc: "温暖自然 · 中英混读首选（默认）".into(),
        },
        VoiceOption {
            id: "zh-CN-YunxiNeural".into(),
            name: "云希 (Yunxi)".into(),
            lang: "zh-CN".into(),
            gender: "男声".into(),
            desc: "阳光清朗青年音 · 适合解说与对话".into(),
        },
        VoiceOption {
            id: "zh-CN-XiaoyiNeural".into(),
            name: "晓伊 (Xiaoyi)".into(),
            lang: "zh-CN".into(),
            gender: "女声".into(),
            desc: "清脆活泼女声 · 富有表现力".into(),
        },
        VoiceOption {
            id: "zh-CN-YunjianNeural".into(),
            name: "云健 (Yunjian)".into(),
            lang: "zh-CN".into(),
            gender: "男声".into(),
            desc: "沉稳磁性男声 · 适合长文朗读".into(),
        },
        VoiceOption {
            id: "zh-CN-YunyangNeural".into(),
            name: "云扬 (Yunyang)".into(),
            lang: "zh-CN".into(),
            gender: "男声".into(),
            desc: "专业新闻播音腔 · 字正腔圆".into(),
        },
        VoiceOption {
            id: "zh-CN-liaoning-XiaobeiNeural".into(),
            name: "晓北 · 东北话".into(),
            lang: "zh-CN".into(),
            gender: "女声".into(),
            desc: "幽默亲切东北方言女声".into(),
        },
        VoiceOption {
            id: "zh-CN-shaanxi-XiaoniNeural".into(),
            name: "晓妮 · 陕西话".into(),
            lang: "zh-CN".into(),
            gender: "女声".into(),
            desc: "地道关中方言特色女声".into(),
        },
        VoiceOption {
            id: "zh-TW-HsiaoChenNeural".into(),
            name: "晓臻 · 台湾女声".into(),
            lang: "zh-TW".into(),
            gender: "女声".into(),
            desc: "温柔甜美台湾国语女声".into(),
        },
        VoiceOption {
            id: "zh-HK-HiuGaaiNeural".into(),
            name: "晓佳 · 粤语女声".into(),
            lang: "zh-HK".into(),
            gender: "女声".into(),
            desc: "自然流畅香港粤语女声".into(),
        },
        VoiceOption {
            id: "en-US-EmmaMultilingualNeural".into(),
            name: "Emma · 美式多语种".into(),
            lang: "en-US".into(),
            gender: "女声".into(),
            desc: "微软旗舰多语言自然女声".into(),
        },
        VoiceOption {
            id: "en-US-AriaNeural".into(),
            name: "Aria · 美式英语".into(),
            lang: "en-US".into(),
            gender: "女声".into(),
            desc: "地道知性美式英语女声".into(),
        },
        VoiceOption {
            id: "en-US-GuyNeural".into(),
            name: "Guy · 美式男声".into(),
            lang: "en-US".into(),
            gender: "男声".into(),
            desc: "自然磁性美式英语男声".into(),
        },
        VoiceOption {
            id: "en-GB-SoniaNeural".into(),
            name: "Sonia · 英式英语".into(),
            lang: "en-GB".into(),
            gender: "女声".into(),
            desc: "优雅纯正英伦女声".into(),
        },
        VoiceOption {
            id: "ja-JP-NanamiNeural".into(),
            name: "七海 Nanami · 日语".into(),
            lang: "ja-JP".into(),
            gender: "女声".into(),
            desc: "自然元气日语标准女声".into(),
        },
        VoiceOption {
            id: "ko-KR-SunHiNeural".into(),
            name: "善姬 SunHi · 韩语".into(),
            lang: "ko-KR".into(),
            gender: "女声".into(),
            desc: "温柔自然韩语标准女声".into(),
        },
    ]
}

/// 根据文本内容与配置智能决策发音人（支持纯日文/韩文/英文自动匹配母语音色）
pub fn resolve_voice(text: &str, preferred_voice: &str, auto_lang: bool) -> String {
    let pref = if preferred_voice.trim().is_empty() {
        "zh-CN-XiaoxiaoNeural"
    } else {
        preferred_voice.trim()
    };
    if !auto_lang {
        return pref.to_string();
    }

    let mut total_letters = 0usize;
    let mut cjk_count = 0usize;
    let mut kana_count = 0usize;
    let mut hangul_count = 0usize;
    let mut latin_count = 0usize;

    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_ascii_punctuation() {
            continue;
        }
        total_letters += 1;
        let u = ch as u32;
        if (0x3040..=0x30FF).contains(&u) || (0x31F0..=0x31FF).contains(&u) {
            kana_count += 1;
        } else if (0xAC00..=0xD7AF).contains(&u) || (0x1100..=0x11FF).contains(&u) {
            hangul_count += 1;
        } else if (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u) {
            cjk_count += 1;
        } else if ch.is_ascii_alphabetic() {
            latin_count += 1;
        }
    }

    if total_letters == 0 {
        return pref.to_string();
    }
    if kana_count * 100 / total_letters >= 12 {
        return "ja-JP-NanamiNeural".to_string();
    }
    if hangul_count * 100 / total_letters >= 15 {
        return "ko-KR-SunHiNeural".to_string();
    }
    // 纯英文（无汉字）且拉丁字母占比高时，若用户原本选的是普通中文单语音色，可智能用 EmmaMultilingual 或保持中文音色
    if cjk_count == 0 && latin_count * 100 / total_letters >= 65 && !pref.starts_with("en-") {
        return "en-US-EmmaMultilingualNeural".to_string();
    }
    pref.to_string()
}

// ============================================================================
// rany2/edge-tts DRM v2 (Sec-MS-GEC) 令牌生成算法 1:1 移植
// ============================================================================

const SHA256_INITIAL: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const SHA256_STATE: [u32; 8] = [
    0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A, 0x6C006F49, 0x04BD35AB, 0xFEB0A7E4, 0x03EFFBD2,
];
const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA, 0xA622EE8C, 0xD1DD9491, 0xC0DD25A0, 0xF9E70AB6, 0xF0C31573,
    0x7058CE4E, 0x7188E606, 0x00E5AB33, 0xD258B060, 0x72766564,
];
const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5, 0xD807AA98,
    0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174, 0xE49B69C1, 0xEFBE4786,
    0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA, 0x983E5152, 0xA831C66D, 0xB00327C8,
    0xBF597FC7, 0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967, 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13,
    0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85, 0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3, 0xD192E819,
    0xD6990624, 0xF40E3585, 0x106AA070, 0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A,
    0x5B9CCA4F, 0x682E6FF3, 0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7,
    0xC67178F2,
];
const SHA256_ADJUSTMENT: u32 = 0xC9DC06A7;
const SHA256_SIGMA0_ADJUSTMENT: u32 = 0x059C0B80;
const SHA256_SIGMA1_ADJUSTMENT: u32 = 0x000B92C0;

const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

#[inline]
fn rol16(val: u32) -> u32 {
    val.rotate_left(16)
}

#[inline]
fn sigma0(val: u32) -> u32 {
    val.rotate_right(7) ^ val.rotate_right(18) ^ (val >> 3)
}

#[inline]
fn sigma1(val: u32) -> u32 {
    val.rotate_right(17) ^ val.rotate_right(19) ^ (val >> 10)
}

fn expand_sha256_schedule(words: &mut Vec<u32>, first_word: usize) {
    for index in first_word..64 {
        let x = words[index - 2];
        let y = words[index - 15];
        let w = sigma1(x)
            .wrapping_add(words[index - 7])
            .wrapping_add(sigma0(y))
            .wrapping_add(words[index - 16]);
        words.push(w);
    }
}

fn sha256_rounds(state: [u32; 8], words: &[u32], first_round: usize) -> [u32; 8] {
    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for index in first_round..64 {
        let k = if first_round == 14 && index == 14 {
            SHA256_ADJUSTMENT
        } else {
            SHA256_ROUND_CONSTANTS[index]
        };
        let big_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = g ^ (e & (f ^ g));
        let t1 = h
            .wrapping_add(big_sigma1)
            .wrapping_add(ch)
            .wrapping_add(k)
            .wrapping_add(words[index]);
        let big_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) | (c & (a | b));
        let t2 = big_sigma0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    [a, b, c, d, e, f, g, h]
}

fn sha256_init(data7: &[u8]) -> [u32; 8] {
    let mut words = Vec::with_capacity(64);
    words.resize(14, 0u32);
    words.push(u32::from_be_bytes([0, data7[0], data7[1], data7[2]]));
    words.push(u32::from_be_bytes([data7[3], data7[4], data7[5], data7[6]]));

    for index in 16..30 {
        let mut value = SHA256_SCHEDULE[index - 16];
        if index - 2 >= 14 {
            let s1 = if index == 16 {
                sigma1(words[index - 2]) ^ SHA256_SIGMA1_ADJUSTMENT
            } else {
                sigma1(words[index - 2])
            };
            value = value.wrapping_add(s1);
        }
        if index - 7 >= 14 {
            value = value.wrapping_add(words[index - 7]);
        }
        if index - 15 >= 14 {
            let s0 = if index == 29 {
                sigma0(words[index - 15]) ^ SHA256_SIGMA0_ADJUSTMENT
            } else {
                sigma0(words[index - 15])
            };
            value = value.wrapping_add(s0);
        }
        words.push(value);
    }

    words.push(
        sigma1(words[28])
            .wrapping_add(words[23])
            .wrapping_add(sigma0(words[15]))
            .wrapping_add(words[14])
            .wrapping_add(SHA256_ADJUSTMENT)
            .wrapping_sub(SHA256_ROUND_CONSTANTS[14]),
    );
    expand_sha256_schedule(&mut words, 31);
    let working = sha256_rounds(SHA256_STATE, &words, 14);
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = SHA256_INITIAL[i].wrapping_add(working[i]);
    }
    out
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let length = 57 + data.len();
    let pad_zeros = (55isize - (length as isize)).rem_euclid(64) as usize;
    let mut padded = Vec::with_capacity(data.len().saturating_sub(7) + 1 + pad_zeros + 8);
    padded.extend_from_slice(&data[7..]);
    padded.push(0x80);
    padded.resize(padded.len() + pad_zeros, 0x00);
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());

    let mut state = sha256_init(&data[..7]);
    for chunk in padded.chunks_exact(64) {
        let mut words = Vec::with_capacity(64);
        for w in chunk.chunks_exact(4) {
            words.push(u32::from_be_bytes([w[0], w[1], w[2], w[3]]));
        }
        expand_sha256_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(working[i]);
        }
    }

    let mut out = [0u8; 32];
    for (i, w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

fn words_le4_even(data: &[u8]) -> Vec<u32> {
    if data.is_empty() {
        return Vec::new();
    }
    let rem = (4 - (data.len() % 4)) % 4;
    let mut padded = Vec::with_capacity(data.len() + rem);
    padded.extend_from_slice(data);
    padded.resize(padded.len() + rem, 0);
    let mut words: Vec<u32> = padded
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) | 1;
    let factors = [key_a.wrapping_mul(CS64_FACTOR), key_b.wrapping_mul(CS64_FACTOR)];

    let mut terms: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .enumerate()
        .map(|(i, (c, _))| c.wrapping_mul(factors[i % 2]))
        .collect();
    let mut offsets: Vec<u32> = CS64_COEFFICIENTS.iter().map(|(_, o)| *o).collect();

    let mut prefixed = Vec::with_capacity(1 + data.len());
    prefixed.push(0x00);
    prefixed.extend_from_slice(data);

    for (index, word) in words_le4_even(&prefixed).into_iter().enumerate() {
        let mut term = word.wrapping_mul(if index % 2 == 0 { key_a } else { key_b });
        let mut offset = rol16(word).wrapping_mul(if index % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }

    let mut swap_acc = 0u32;
    let mut swap_last = 0u32;
    let mut state = 0u32;
    let mut acc = 0u32;

    for index in (0..terms.len()).step_by(2) {
        let x = terms[index];
        let y = terms[index + 1];

        let t = rol16(offsets[index].wrapping_add(x));
        let q = t.wrapping_mul(0x76F50F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));

        let t = rol16(offsets[index + 1].wrapping_add(y));
        let q = t.wrapping_mul(0x1881CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98B111);
        state = rol16(state).wrapping_mul(0x87085B9F);
        state = rol16(state).wrapping_mul(0x12CEB96D);
        state = rol16(state).wrapping_mul(0x257E1D83);
        acc = acc.wrapping_add(state);

        let mut t = state.wrapping_mul(key_b).wrapping_add(y);
        t = rol16(t).wrapping_mul(0xA27416F5);
        t = rol16(t).wrapping_mul(0xD38396FF);
        state = rol16(rol16(t).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        acc = acc.wrapping_add(state);
    }

    (swap_last ^ state, swap_acc ^ acc)
}

fn to_upper_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

/// 生成 Edge TTS v2 `Sec-MS-GEC` 校验码
pub fn generate_sec_ms_gec_v2(unix_ts_secs: i64) -> String {
    let mut ticks = unix_ts_secs + WIN_EPOCH;
    ticks -= ticks.rem_euclid(300);
    let ticks_100ns = ticks * 10_000_000;
    let str_to_hash = format!("{ticks_100ns}{TRUSTED_CLIENT_TOKEN}");
    let data = str_to_hash.as_bytes();
    let digest = sha256_digest(data);
    let (c0, c1) = cs64_digest(data, &digest);
    let mut cs64_bytes = [0u8; 8];
    cs64_bytes[0..4].copy_from_slice(&c0.to_le_bytes());
    cs64_bytes[4..8].copy_from_slice(&c1.to_le_bytes());
    format!("{}-{}", to_upper_hex(&cs64_bytes), to_upper_hex(&digest))
}

fn current_adjusted_unix_secs() -> i64 {
    let local = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    local + CLOCK_SKEW_SECS.load(Ordering::Relaxed)
}

fn random_hex_32() -> String {
    let seq = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let a = nanos.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(seq);
    let b = a.rotate_left(29).wrapping_mul(0xBF58476D1CE4E5B9) ^ seq.wrapping_mul(0x94D049BB133111EB);
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..16].copy_from_slice(&b.to_be_bytes());
    to_upper_hex(&bytes)
}

fn js_date_string() -> String {
    let ts = current_adjusted_unix_secs();
    let dt = chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(chrono::Utc::now);
    dt.format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

// ============================================================================
// 文本清洗、XML 转义与分段
// ============================================================================

fn escape_ssml_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 32);
    for ch in raw.chars() {
        let code = ch as u32;
        if (0..=8).contains(&code) || (11..=12).contains(&code) || (14..=31).contains(&code) {
            out.push(' ');
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn split_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.chars().count() <= max_chars {
        return vec![trimmed.to_string()];
    }
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;

    for ch in trimmed.chars() {
        cur.push(ch);
        cur_len += 1;
        let is_break = matches!(ch, '。' | '！' | '？' | '；' | '\n' | '.' | '!' | '?' | ';');
        if (is_break && cur_len >= max_chars / 2) || cur_len >= max_chars {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                chunks.push(s);
            }
            cur.clear();
            cur_len = 0;
        }
    }
    if !cur.trim().is_empty() {
        chunks.push(cur.trim().to_string());
    }
    chunks
}

// ============================================================================
// RFC 6455 WebSocket 帧读写与 Edge TTS 通信
// ============================================================================

async fn ws_write_text_frame<S: AsyncWriteExt + Unpin>(stream: &mut S, text: &str) -> Result<()> {
    let payload = text.as_bytes();
    let len = payload.len();
    let mask_key: [u8; 4] = [0x37, 0xFA, 0x21, 0x3D];

    let mut frame = Vec::with_capacity(14 + len);
    frame.push(0x81); // FIN=1, Opcode=1 (Text)
    if len < 126 {
        frame.push(0x80 | (len as u8));
    } else if len <= 65535 {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask_key);
    for (i, &b) in payload.iter().enumerate() {
        frame.push(b ^ mask_key[i & 3]);
    }
    stream.write_all(&frame).await?;
    stream.flush().await?;
    Ok(())
}

async fn ws_read_frame<S: AsyncReadExt + Unpin>(stream: &mut S) -> Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    let opcode = head[0] & 0x0F;
    let masked = (head[1] & 0x80) != 0;
    let mut len = (head[1] & 0x7F) as usize;

    if len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext).await?;
        len = u16::from_be_bytes(ext) as usize;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext).await?;
        len = u64::from_be_bytes(ext) as usize;
    }

    let mut mask_key = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask_key).await?;
    }

    let mut payload = vec![0u8; len];
    if len > 0 {
        stream.read_exact(&mut payload).await?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask_key[i & 3];
            }
        }
    }
    Ok((opcode, payload))
}

fn tls_connector() -> Result<TlsConnector> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let cfg = CONFIG.get_or_init(|| {
        let mut root_store = RootCertStore::empty();
        root_store.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Arc::new(config)
    });
    Ok(TlsConnector::from(cfg.clone()))
}

enum HandshakeError {
    ClockSkewAdjusted,
    Other(anyhow::Error),
}

async fn synthesize_chunk_once(
    escaped_ssml_text: &str,
    voice: &str,
    rate: &str,
    volume: &str,
    pitch: &str,
) -> std::result::Result<Vec<u8>, HandshakeError> {
    let conn_id = random_hex_32().to_lowercase();
    let muid = random_hex_32();
    let gec = generate_sec_ms_gec_v2(current_adjusted_unix_secs());

    let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect((HOST, PORT)))
        .await
        .map_err(|_| HandshakeError::Other(anyhow!("连接微软 Edge TTS 服务器超时")))?
        .map_err(|e| HandshakeError::Other(anyhow!("TCP 连接失败: {e}")))?;

    let connector = tls_connector().map_err(HandshakeError::Other)?;
    let domain = ServerName::try_from(HOST).map_err(|e| HandshakeError::Other(anyhow!("域名无效: {e}")))?;
    let mut tls = tokio::time::timeout(Duration::from_secs(10), connector.connect(domain, tcp))
        .await
        .map_err(|_| HandshakeError::Other(anyhow!("TLS 握手超时")))?
        .map_err(|e| HandshakeError::Other(anyhow!("TLS 握手失败: {e}")))?;

    let req = format!(
        "GET /consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken={TRUSTED_CLIENT_TOKEN}&ConnectionId={conn_id}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION} HTTP/1.1\r\n\
         Host: {HOST}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Pragma: no-cache\r\n\
         Cache-Control: no-cache\r\n\
         Origin: chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold\r\n\
         User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{CHROMIUM_MAJOR_VERSION}.0.0.0 Safari/537.36 Edg/{CHROMIUM_MAJOR_VERSION}.0.0.0\r\n\
         Accept-Encoding: gzip, deflate, br, zstd\r\n\
         Accept-Language: en-US,en;q=0.9\r\n\
         Cookie: muid={muid};\r\n\r\n"
    );

    tls.write_all(req.as_bytes())
        .await
        .map_err(|e| HandshakeError::Other(anyhow!("发送握手失败: {e}")))?;
    tls.flush()
        .await
        .map_err(|e| HandshakeError::Other(anyhow!("刷新握手缓冲失败: {e}")))?;

    // 读取 HTTP 响应头直到 \r\n\r\n
    let mut resp_buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while resp_buf.len() < 8192 {
        let n = tokio::time::timeout(Duration::from_secs(10), tls.read(&mut byte))
            .await
            .map_err(|_| HandshakeError::Other(anyhow!("读取 WebSocket 握手响应超时")))?
            .map_err(|e| HandshakeError::Other(anyhow!("读取握手响应失败: {e}")))?;
        if n == 0 {
            break;
        }
        resp_buf.push(byte[0]);
        if resp_buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let resp_str = String::from_utf8_lossy(&resp_buf);
    let first_line = resp_str.lines().next().unwrap_or("");
    if !first_line.contains(" 101 ") {
        // 检查是否有 Date 响应头用于自动校准本机时钟偏差
        for line in resp_str.lines() {
            if let Some(rest) = line.strip_prefix("Date:").or_else(|| line.strip_prefix("date:")) {
                let date_str = rest.trim();
                if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(date_str) {
                    let server_ts = dt.timestamp();
                    let local_ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let new_skew = server_ts - local_ts;
                    let old_skew = CLOCK_SKEW_SECS.swap(new_skew, Ordering::SeqCst);
                    if (new_skew - old_skew).abs() > 5 {
                        return Err(HandshakeError::ClockSkewAdjusted);
                    }
                }
            }
        }
        return Err(HandshakeError::Other(anyhow!("Edge TTS 握手被拒绝: {first_line}")));
    }

    // 1. 发送 speech.config
    let ts = js_date_string();
    let config_msg = format!(
        "X-Timestamp:{ts}\r\n\
         Content-Type:application/json; charset=utf-8\r\n\
         Path:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n"
    );
    ws_write_text_frame(&mut tls, &config_msg)
        .await
        .map_err(HandshakeError::Other)?;

    // 2. 发送 SSML
    let req_id = random_hex_32().to_lowercase();
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'>\
         <prosody pitch='{pitch}' rate='{rate}' volume='{volume}'>{escaped_ssml_text}</prosody>\
         </voice>\
         </speak>"
    );
    let ssml_msg = format!(
        "X-RequestId:{req_id}\r\n\
         Content-Type:application/ssml+xml\r\n\
         X-Timestamp:{ts}Z\r\n\
         Path:ssml\r\n\r\n\
         {ssml}"
    );
    ws_write_text_frame(&mut tls, &ssml_msg)
        .await
        .map_err(HandshakeError::Other)?;

    // 3. 接收音频流直到 Path:turn.end
    let mut audio_bytes = Vec::with_capacity(32 * 1024);
    let mut frag_opcode = 0u8;
    let mut frag_buf = Vec::new();

    loop {
        let (opcode, data) = tokio::time::timeout(Duration::from_secs(25), ws_read_frame(&mut tls))
            .await
            .map_err(|_| HandshakeError::Other(anyhow!("接收语音数据流超时")))?
            .map_err(HandshakeError::Other)?;

        let actual_opcode = if opcode == 0 {
            frag_buf.extend_from_slice(&data);
            frag_opcode
        } else if opcode == 1 || opcode == 2 {
            frag_opcode = opcode;
            frag_buf.clear();
            frag_buf.extend_from_slice(&data);
            opcode
        } else {
            opcode
        };

        match actual_opcode {
            0x1 => {
                // 文本帧
                let txt = String::from_utf8_lossy(&frag_buf);
                if txt.contains("Path:turn.end") {
                    break;
                }
            }
            0x2 => {
                // 二进制帧：前 2 字节大端为 header 长度
                if frag_buf.len() >= 2 {
                    let header_len = u16::from_be_bytes([frag_buf[0], frag_buf[1]]) as usize;
                    if 2 + header_len <= frag_buf.len() {
                        let header = &frag_buf[2..2 + header_len];
                        let header_str = String::from_utf8_lossy(header);
                        if header_str.contains("Path:audio") {
                            let body = &frag_buf[2 + header_len..];
                            if !body.is_empty() {
                                audio_bytes.extend_from_slice(body);
                            }
                        }
                    }
                }
            }
            0x8 => {
                // Close
                break;
            }
            0x9 => {
                // Ping -> Pong
                let mut pong = vec![0x8A, 0x80, 0x11, 0x22, 0x33, 0x44];
                let _ = tls.write_all(&pong).await;
                pong.clear();
            }
            _ => {}
        }
    }

    if audio_bytes.is_empty() {
        return Err(HandshakeError::Other(anyhow!("未收到有效语音音频流")));
    }

    Ok(audio_bytes)
}

/// 合成单段或长文本为 `data:audio/mp3;base64,...` Data URI
pub async fn synthesize(text: &str, voice: &str, rate: &str, volume: &str) -> Result<String> {
    let clean = text.trim();
    if clean.is_empty() {
        return Err(anyhow!("朗读文本为空"));
    }

    let resolved_voice = if voice.trim().is_empty() {
        "zh-CN-XiaoxiaoNeural"
    } else {
        voice.trim()
    };
    let rate_str = if rate.trim().is_empty() { "+0%" } else { rate.trim() };
    let vol_str = if volume.trim().is_empty() { "+0%" } else { volume.trim() };
    let pitch_str = "+0Hz";

    let cache_key = format!("{resolved_voice}|{rate_str}|{vol_str}|{clean}");
    if let Some(cached) = cache_get(&cache_key) {
        return Ok(cached);
    }

    let chunks = split_chunks(clean, 1200);
    let mut combined_mp3 = Vec::new();

    for chunk in chunks {
        let escaped = escape_ssml_text(&chunk);
        if escaped.trim().is_empty() {
            continue;
        }
        let mp3 = match synthesize_chunk_once(&escaped, resolved_voice, rate_str, vol_str, pitch_str).await {
            Ok(bytes) => bytes,
            Err(HandshakeError::ClockSkewAdjusted) => {
                // 时钟偏差已自动校准，立即重试一次
                synthesize_chunk_once(&escaped, resolved_voice, rate_str, vol_str, pitch_str)
                    .await
                    .map_err(|e| match e {
                        HandshakeError::ClockSkewAdjusted => anyhow!("时钟校准后握手仍失败"),
                        HandshakeError::Other(err) => err,
                    })?
            }
            Err(HandshakeError::Other(err)) => return Err(err),
        };
        combined_mp3.extend_from_slice(&mp3);
    }

    if combined_mp3.is_empty() {
        return Err(anyhow!("未能生成任何语音数据"));
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&combined_mp3);
    let data_uri = format!("data:audio/mp3;base64,{b64}");
    cache_put(cache_key, data_uri.clone());
    Ok(data_uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sec_ms_gec_v2_vector() {
        let gec = generate_sec_ms_gec_v2(1_730_000_000);
        assert_eq!(
            gec,
            "50065ABF67B90F95-49BDAF24D28ECEB38DAEEC84DEAA71E91F92945EA8A47921C58FB4F8F79A4F5E"
        );
    }

    #[tokio::test]
    async fn test_live_edge_tts_synthesize() {
        let res = synthesize("你好，我是 miniBot 语音助手。", "zh-CN-XiaoxiaoNeural", "+0%", "+0%").await;
        match res {
            Ok(uri) => {
                println!("Edge TTS synthesis succeeded! Data URI len = {}", uri.len());
                assert!(uri.starts_with("data:audio/mp3;base64,"));
                assert!(uri.len() > 1000);
            }
            Err(e) => {
                panic!("Edge TTS synthesis failed: {e}");
            }
        }
    }
}
