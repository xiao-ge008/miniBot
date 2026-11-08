//! 以图搜图（反向图片搜索）聚合引擎：
//! 参考 kitUIN/PicImageSearch 与 eSearch 协议，纯 Rust 原生免 Key 实现：
//! 1. 百度识图 (graph.baidu.com)：国内毫秒级直连，解析 window.cardData 提取 AI 猜词、同款原图出处 (same) 与 30 张相似图片瀑布流 (simipic)
//! 2. AnimeTrace (api.animetrace.com)：国内直连动漫/游戏角色识别，支持原图角色区域自动裁切
//! 3. Trace.moe (api.trace.moe)：动漫番剧截图搜番（定位集数、精确时间点与预览画面）
//! 4. Yandex 图床中转与全网搜图直达链接生成（百度 / 必应 / Yandex / Google Lens / SauceNAO / Ascii2D）

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSearchExternalUrl {
    pub id: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSearchItem {
    pub title: String,
    pub subtitle: String,
    pub image_url: String,
    pub source_url: String,
    pub badge: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSearchResponse {
    pub engine: String,
    pub guess: String,
    pub page_url: String,
    pub public_image_url: String,
    pub external_urls: Vec<ImageSearchExternalUrl>,
    pub items: Vec<ImageSearchItem>,
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn extract_host(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    rest.split('/')
        .next()
        .unwrap_or("")
        .strip_prefix("www.")
        .unwrap_or(rest.split('/').next().unwrap_or(""))
        .to_string()
}

/// 手动构造标准 multipart/form-data 请求体，零额外 crate 依赖
fn build_multipart(
    fields: &[(&str, &str)],
    file_field: &str,
    file_name: &str,
    content_type: &str,
    file_bytes: &[u8],
) -> (String, Vec<u8>) {
    let boundary = format!("----miniBotFormBoundary{}", crate::payload::new_id());
    let mut body = Vec::with_capacity(file_bytes.len() + 512);

    for (k, v) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes(),
        );
    }

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{file_field}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn direct_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_default()
}

fn auto_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_default()
}

/// 根据公网可访问的图片 URL 与百度结果页 URL，构建全网搜图引擎一键直达链接
fn build_external_urls(baidu_page_url: &str, public_img_url: &str) -> Vec<ImageSearchExternalUrl> {
    let mut out = Vec::new();
    if !baidu_page_url.is_empty() {
        out.push(ImageSearchExternalUrl {
            id: "baidu".into(),
            name: "百度识图".into(),
            url: baidu_page_url.to_string(),
        });
    }
    if !public_img_url.is_empty() {
        let enc = url_encode(public_img_url);
        out.push(ImageSearchExternalUrl {
            id: "bing".into(),
            name: "必应搜图".into(),
            url: format!(
                "https://cn.bing.com/images/search?view=detailv2&iss=sbi&FORM=SBIHMP&sbisrc=UrlPaste&q=imgurl:{enc}"
            ),
        });
        out.push(ImageSearchExternalUrl {
            id: "yandex".into(),
            name: "Yandex".into(),
            url: format!("https://yandex.com/images/search?rpt=imageview&url={enc}"),
        });
        out.push(ImageSearchExternalUrl {
            id: "google_lens".into(),
            name: "Google Lens".into(),
            url: format!("https://lens.google.com/uploadbyurl?url={enc}"),
        });
        out.push(ImageSearchExternalUrl {
            id: "saucenao".into(),
            name: "SauceNAO".into(),
            url: format!("https://saucenao.com/search.php?url={enc}"),
        });
        out.push(ImageSearchExternalUrl {
            id: "ascii2d".into(),
            name: "Ascii2D".into(),
            url: format!("https://ascii2d.net/search/uri?uri={enc}"),
        });
    }
    out
}

/// 尝试将图片上传至 Yandex 图床获取全球无防盗链的公网 URL（超时 2.5s，失败静默返回空）
async fn try_yandex_public_url(png_bytes: &[u8]) -> Option<String> {
    let client = auto_client(3);
    let url = "https://yandex.com/images-apphost/image-download?cbird=111&images_avatars_size=preview&images_avatars_namespace=images-cbir";
    let resp = client
        .post(url)
        .header("User-Agent", UA)
        .header("Content-Type", "image/png")
        .body(png_bytes.to_vec())
        .send()
        .await
        .ok()?;
    let json: Value = resp.json().await.ok()?;
    json.pointer("/sizes/orig/path")
        .and_then(|v| v.as_str())
        .or_else(|| json.get("url").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

/// 从百度结果页 HTML 中提取 `window.cardData` JSON 数组
fn extract_baidu_card_data(html: &str) -> Vec<Value> {
    let Some(idx) = html.find("window.cardData") else {
        return Vec::new();
    };
    let sub = &html[idx..];
    let Some(start) = sub.find('[') else {
        return Vec::new();
    };
    let after_start = &sub[start..];
    // 找到匹配的 `];` 或 `</script>` 前的最后一个 `]`
    let script_end = after_start.find("</script>").unwrap_or(after_start.len());
    let script_slice = &after_start[..script_end];
    let Some(end) = script_slice.rfind(']') else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<Value>>(&script_slice[..=end]).unwrap_or_default()
}

fn pick_str(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

/// 引擎 1：百度识图（国内直连毫秒级响应，返回猜词 + 同款原图 + 30 张相似图片瀑布流）
async fn search_baidu(png_bytes: &[u8]) -> Result<ImageSearchResponse, String> {
    let client = direct_client(7);
    let (ct, body) = build_multipart(&[("from", "pc")], "image", "image.png", "image/png", png_bytes);

    let upload_resp = client
        .post("https://graph.baidu.com/upload")
        .header("User-Agent", UA)
        .header("Acs-Token", "")
        .header("Content-Type", ct)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("上传图片到百度识图失败：{e}"))?;

    let upload_json: Value = upload_resp
        .json()
        .await
        .map_err(|e| format!("解析百度识图响应失败：{e}"))?;

    let page_url = upload_json
        .pointer("/data/url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if page_url.is_empty() {
        return Err("百度识图未返回有效结果链接".into());
    }

    // 并行拉取百度结果页 HTML 与可选的 Yandex 全球图床直链
    let page_fut = async {
        client
            .get(&page_url)
            .header("User-Agent", UA)
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await
    };
    let (page_res, yandex_url_opt) = tokio::join!(page_fut, try_yandex_public_url(png_bytes));

    let html = match page_res {
        Ok(r) => r.text().await.unwrap_or_default(),
        Err(_) => String::new(),
    };

    let cards = extract_baidu_card_data(&html);
    let mut guess = String::new();
    let mut bos_img_url = String::new();
    let mut items: Vec<ImageSearchItem> = Vec::new();
    let mut simi_first_url = String::new();

    for card in &cards {
        let card_name = card.get("cardName").and_then(|v| v.as_str()).unwrap_or("");
        let tpl = card.get("tplData").unwrap_or(&Value::Null);

        match card_name {
            "cardHeader" => {
                bos_img_url = pick_str(tpl, &["originImgUrl", "imageUrl"]);
                if let Some(pano_list) = tpl.pointer("/pano/list").and_then(|v| v.as_array()) {
                    for p in pano_list {
                        let sub = pick_str(p, &["subTitle", "title"]);
                        if !sub.is_empty() {
                            guess = sub;
                            break;
                        }
                    }
                }
            }
            "same" => {
                if let Some(list) = tpl.get("list").and_then(|v| v.as_array()) {
                    for entry in list {
                        let img_u = pick_str(entry, &["image_src", "thumbUrl", "objUrl"]);
                        let src_u = pick_str(entry, &["url", "fromUrl"]);
                        if img_u.is_empty() {
                            continue;
                        }
                        let title = entry
                            .get("title")
                            .and_then(|t| {
                                t.as_str().map(String::from).or_else(|| {
                                    t.as_array()
                                        .and_then(|arr| arr.first())
                                        .and_then(|x| x.as_str())
                                        .map(String::from)
                                })
                            })
                            .unwrap_or_else(|| pick_str(entry, &["subTitle", "fromPageTitle"]));
                        let host = extract_host(&src_u);
                        let w = entry.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let h = entry.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let subtitle = if w > 0 && h > 0 && !host.is_empty() {
                            format!("{w}×{h} · {host}")
                        } else if !host.is_empty() {
                            host
                        } else {
                            "同款来源".into()
                        };
                        items.push(ImageSearchItem {
                            title: if title.is_empty() { "同款图片来源".into() } else { title },
                            subtitle,
                            image_url: img_u,
                            source_url: if src_u.is_empty() { page_url.clone() } else { src_u },
                            badge: "同款".into(),
                            width: w,
                            height: h,
                        });
                    }
                }
            }
            "simipic" => {
                simi_first_url = pick_str(tpl, &["firstUrl"]);
            }
            _ => {}
        }
    }

    // 拉取相似图片瀑布流列表 (simipic)
    if !simi_first_url.is_empty() {
        if let Ok(resp) = client
            .get(&simi_first_url)
            .header("User-Agent", UA)
            .send()
            .await
        {
            if let Ok(simi_json) = resp.json::<Value>().await {
                if let Some(list) = simi_json.pointer("/data/list").and_then(|v| v.as_array()) {
                    for entry in list {
                        let img_u = pick_str(entry, &["thumbUrl", "image_src", "hoverUrl"]);
                        let src_u = pick_str(entry, &["fromUrl", "url", "objUrl"]);
                        if img_u.is_empty() {
                            continue;
                        }
                        let w = entry.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let h = entry.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let host = extract_host(&src_u);
                        let raw_title = pick_str(entry, &["fromPageTitle", "title", "subTitle"]);
                        let title = if !raw_title.is_empty() {
                            raw_title
                        } else if !host.is_empty() {
                            host.clone()
                        } else {
                            "相似图片".into()
                        };
                        let subtitle = if w > 0 && h > 0 {
                            if !host.is_empty() && title != host {
                                format!("{w} × {h} · {host}")
                            } else {
                                format!("{w} × {h}")
                            }
                        } else {
                            host
                        };
                        items.push(ImageSearchItem {
                            title,
                            subtitle,
                            image_url: img_u,
                            source_url: if src_u.is_empty() { page_url.clone() } else { src_u },
                            badge: "相似".into(),
                            width: w,
                            height: h,
                        });
                    }
                }
            }
        }
    }

    let public_image_url = yandex_url_opt.unwrap_or(bos_img_url);
    let external_urls = build_external_urls(&page_url, &public_image_url);

    Ok(ImageSearchResponse {
        engine: "baidu".into(),
        guess,
        page_url,
        public_image_url,
        external_urls,
        items,
    })
}

/// 引擎 2：Trace.moe 动漫番剧截图搜番（定位番剧名、集数、精确时间点与画面截图）
async fn search_tracemoe(png_bytes: &[u8]) -> Result<ImageSearchResponse, String> {
    let client = auto_client(10);
    let (ct, body) = build_multipart(&[], "image", "image.png", "image/png", png_bytes);

    let resp = client
        .post("https://api.trace.moe/search?anilistInfo")
        .header("User-Agent", UA)
        .header("Content-Type", ct)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("连接 Trace.moe 搜番服务失败：{e}"))?;

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 Trace.moe 响应失败：{e}"))?;

    let mut items = Vec::new();
    let mut best_guess = String::new();

    if let Some(list) = json.get("result").and_then(|v| v.as_array()) {
        for (idx, entry) in list.iter().enumerate() {
            let image_url = pick_str(entry, &["image"]);
            if image_url.is_empty() {
                continue;
            }
            let sim = entry
                .get("similarity")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                * 100.0;
            let ep_str = match entry.get("episode") {
                Some(Value::Number(n)) => format!("第 {n} 集"),
                Some(Value::String(s)) if !s.is_empty() => format!("第 {s} 集"),
                _ => "正片/剧场".into(),
            };
            let from_sec = entry.get("from").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
            let mm = from_sec / 60;
            let ss = from_sec % 60;

            let anilist = entry.get("anilist").unwrap_or(&Value::Null);
            let title_obj = anilist.get("title").unwrap_or(&Value::Null);
            let title = pick_str(title_obj, &["chinese", "native", "romaji", "english"]);
            let final_title = if !title.is_empty() {
                title
            } else {
                pick_str(entry, &["filename"])
            };

            let anilist_id = anilist.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let source_url = if anilist_id > 0 {
                format!("https://anilist.co/anime/{anilist_id}")
            } else {
                format!(
                    "https://cn.bing.com/search?q={}",
                    url_encode(&final_title)
                )
            };

            if idx == 0 {
                best_guess = format!("{final_title} · {ep_str} {mm:02}:{ss:02} ({sim:.1}%)");
            }

            items.push(ImageSearchItem {
                title: final_title,
                subtitle: format!("{ep_str} · {mm:02}:{ss:02}"),
                image_url,
                source_url,
                badge: format!("{sim:.1}%"),
                width: 320,
                height: 180,
            });
        }
    }

    Ok(ImageSearchResponse {
        engine: "tracemoe".into(),
        guess: best_guess,
        page_url: "https://trace.moe".into(),
        public_image_url: String::new(),
        external_urls: Vec::new(),
        items,
    })
}

/// 引擎 3：AnimeTrace 动漫 / 游戏角色识别（国内直连，自动从原图裁切识别出的角色头像）
async fn search_animetrace(png_bytes: &[u8]) -> Result<ImageSearchResponse, String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    let client = direct_client(10);

    let payload = serde_json::json!({
        "base64": b64,
        "is_multi": 1,
        "ai_detect": 1
    });

    let resp = client
        .post("https://api.animetrace.com/v1/search")
        .header("User-Agent", UA)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("连接 AnimeTrace 角色识别服务失败：{e}"))?;

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 AnimeTrace 响应失败：{e}"))?;

    let orig_rgba = crate::imgutil::decode_rgba(png_bytes).ok();
    let full_data_uri = crate::imgutil::to_data_uri(png_bytes);

    let mut items = Vec::new();
    let mut best_guess = String::new();

    if let Some(data_arr) = json.get("data").and_then(|v| v.as_array()) {
        for box_entry in data_arr {
            // 根据归一化坐标 [x1, y1, x2, y2] 裁切角色区域图片
            let mut crop_uri = full_data_uri.clone();
            let mut cw = 240u32;
            let mut ch = 240u32;
            if let (Some(img), Some(b)) = (&orig_rgba, box_entry.get("box").and_then(|v| v.as_array())) {
                if b.len() == 4 {
                    let x1 = b[0].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
                    let y1 = b[1].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
                    let x2 = b[2].as_f64().unwrap_or(1.0).clamp(0.0, 1.0);
                    let y2 = b[3].as_f64().unwrap_or(1.0).clamp(0.0, 1.0);
                    let iw = img.width();
                    let ih = img.height();
                    let px = (x1 * iw as f64) as u32;
                    let py = (y1 * ih as f64) as u32;
                    let pw = ((x2 - x1) * iw as f64).round().max(16.0) as u32;
                    let ph = ((y2 - y1) * ih as f64).round().max(16.0) as u32;
                    let pw = pw.min(iw.saturating_sub(px));
                    let ph = ph.min(ih.saturating_sub(py));
                    if pw >= 16 && ph >= 16 {
                        let sub = image::imageops::crop_imm(img, px, py, pw, ph).to_image();
                        let cropped_png = crate::imgutil::encode_png(&sub);
                        crop_uri = crate::imgutil::to_data_uri(&cropped_png);
                        cw = pw;
                        ch = ph;
                    }
                }
            }

            if let Some(chars) = box_entry.get("character").and_then(|v| v.as_array()) {
                for (i, ch_item) in chars.iter().take(6).enumerate() {
                    let name = pick_str(ch_item, &["character"]);
                    let work = pick_str(ch_item, &["work"]);
                    if name.is_empty() {
                        continue;
                    }
                    if best_guess.is_empty() {
                        best_guess = if work.is_empty() {
                            name.clone()
                        } else {
                            format!("{name}（《{work}》）")
                        };
                    }
                    let q = if work.is_empty() {
                        name.clone()
                    } else {
                        format!("{work} {name}")
                    };
                    items.push(ImageSearchItem {
                        title: name,
                        subtitle: if work.is_empty() { "动漫/游戏角色".into() } else { format!("《{work}》") },
                        image_url: crop_uri.clone(),
                        source_url: format!("https://cn.bing.com/images/search?q={}", url_encode(&q)),
                        badge: if i == 0 { "首选角色".into() } else { format!("候选 #{}", i + 1) },
                        width: cw,
                        height: ch,
                    });
                }
            }
        }
    }

    Ok(ImageSearchResponse {
        engine: "animetrace".into(),
        guess: best_guess,
        page_url: "https://www.animetrace.com".into(),
        public_image_url: String::new(),
        external_urls: Vec::new(),
        items,
    })
}

/// 统一搜图入口：`engine` 支持 `"baidu"` | `"tracemoe"` | `"animetrace"`
pub async fn search_by_image(data_b64: &str, engine: &str) -> Result<ImageSearchResponse, String> {
    let raw_bytes = crate::imgutil::decode_b64(data_b64).map_err(|e| e.to_string())?;
    // 统一解码并转为标准 PNG（若图片过大则等比缩放到长边 <= 1280，加快上传速度）
    let rgba = crate::imgutil::decode_rgba(&raw_bytes).map_err(|e| e.to_string())?;
    let max_dim = rgba.width().max(rgba.height());
    let png_bytes = if max_dim > 1280 {
        let ratio = 1280.0 / max_dim as f64;
        let nw = ((rgba.width() as f64 * ratio).round() as u32).max(1);
        let nh = ((rgba.height() as f64 * ratio).round() as u32).max(1);
        let resized = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Triangle);
        crate::imgutil::encode_png(&resized)
    } else {
        crate::imgutil::encode_png(&rgba)
    };

    match engine {
        "tracemoe" => search_tracemoe(&png_bytes).await,
        "animetrace" => search_animetrace(&png_bytes).await,
        _ => search_baidu(&png_bytes).await,
    }
}

/// 拉取远程图片 URL 并转换为标准 `data:image/png;base64,...` Data URI，
/// 供前端对搜出来的单张图片执行「保存图片」或「发送到 AI 聊天」
pub async fn fetch_remote_as_data_uri(url: &str) -> Result<String, String> {
    if url.starts_with("data:image/") {
        return Ok(url.to_string());
    }
    let is_domestic = url.contains(".baidu.com") || url.contains(".bdimg.com") || url.contains(".bcebos.com");
    let client = if is_domestic {
        direct_client(8)
    } else {
        auto_client(10)
    };

    let resp = client
        .get(url)
        .header("User-Agent", UA)
        .send()
        .await
        .map_err(|e| format!("下载图片失败：{e}"))?;

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取图片数据失败：{e}"))?;

    let rgba = crate::imgutil::decode_rgba(&bytes)
        .map_err(|e| format!("解码图片格式失败：{e}"))?;
    let png = crate::imgutil::encode_png(&rgba);
    Ok(crate::imgutil::to_data_uri(&png))
}
