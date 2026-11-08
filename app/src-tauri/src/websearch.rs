//! 免费免 Key 联网搜索引擎：
//! 多源自动聚合与故障转移（Bing 国内直连 + 360 搜索 + DuckDuckGo HTML），
//! 供主 AI 聊天窗、划词悬浮窗及截图问答实时检索互联网最新信息。

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub date: String,
    pub hits: Vec<SearchHit>,
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let raw = s.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&raw[i + 1..i + 3]).unwrap_or(""), 16) {
                bytes.push(v);
                i += 3;
                continue;
            }
        } else if raw[i] == b'+' {
            bytes.push(b' ');
            i += 1;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 剥离 HTML 标签并解码常见 HTML 实体
fn strip_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#183;", "·")
        .replace("&mdash;", "—");
    // 合并多余空白字符
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 从标签片段中提取指定属性值（如 href="..."）
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let pat_dq = format!("{attr}=\"");
    if let Some(i) = tag.find(&pat_dq) {
        let rest = &tag[i + pat_dq.len()..];
        if let Some(j) = rest.find('"') {
            return Some(rest[..j].replace("&amp;", "&"));
        }
    }
    let pat_sq = format!("{attr}='");
    if let Some(i) = tag.find(&pat_sq) {
        let rest = &tag[i + pat_sq.len()..];
        if let Some(j) = rest.find('\'') {
            return Some(rest[..j].replace("&amp;", "&"));
        }
    }
    None
}

/// 清理关键词：提炼用户长句中的核心搜索词
pub fn clean_query(raw: &str) -> String {
    let trimmed = raw.trim();
    // 只取第一行或前 80 个字符，避免把整段长文章塞进搜索引擎 URL
    let first_line = trimmed
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(trimmed);
    let chars: String = first_line.chars().take(72).collect();
    chars
}

/// 引擎 1：Bing 搜索（cn.bing.com，国内直连免 Key）
async fn search_bing(client: &reqwest::Client, query: &str, max_results: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let encoded = url_encode(query);
    let url = format!("https://cn.bing.com/search?q={encoded}&count=10");

    let Ok(resp) = client
        .get(&url)
        .header("User-Agent", UA)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Cookie", "SRCHHPGUSR=SRCHLANG=zh-Hans;")
        .send()
        .await
    else {
        return hits;
    };

    let Ok(html) = resp.text().await else {
        return hits;
    };

    for block in html.split("<li class=\"b_algo\"").skip(1) {
        let end = block.find("</li>").unwrap_or(block.len());
        let item = &block[..end];

        // 提取 <h2>...<a href="...">标题</a>...</h2>
        let Some(h2_start) = item.find("<h2") else { continue };
        let after_h2 = &item[h2_start..];
        let Some(h2_end) = after_h2.find("</h2>") else { continue };
        let h2_html = &after_h2[..h2_end];

        let Some(a_start) = h2_html.find("<a ") else { continue };
        let a_html = &h2_html[a_start..];
        let Some(link) = extract_attr(a_html, "href") else { continue };
        if !link.starts_with("http://") && !link.starts_with("https://") {
            continue;
        }
        let title = strip_html(h2_html);
        if title.is_empty() {
            continue;
        }

        // 提取摘要段落
        let mut snippet = String::new();
        if let Some(p_idx) = item.find("<p") {
            let after_p = &item[p_idx..];
            if let Some(p_end) = after_p.find("</p>") {
                snippet = strip_html(&after_p[..p_end]);
            }
        }
        if snippet.is_empty() {
            if let Some(cap_idx) = item.find("b_caption") {
                let after_cap = &item[cap_idx..];
                let cap_end = after_cap.find("</div>").unwrap_or(after_cap.len().min(600));
                snippet = strip_html(&after_cap[..cap_end]);
            }
        }
        // 去除摘要前缀中的冗余日期说明后截断到 220 字
        let snippet_clean: String = snippet.chars().take(220).collect();
        if snippet_clean.is_empty() {
            continue;
        }

        hits.push(SearchHit {
            title,
            url: link,
            snippet: snippet_clean,
            source: "Bing".into(),
        });
        if hits.len() >= max_results {
            break;
        }
    }

    hits
}

/// 引擎 2：360 网页搜索（www.so.com，国内直连免 Key 备用）
async fn search_so360(client: &reqwest::Client, query: &str, max_results: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let encoded = url_encode(query);
    let url = format!("https://www.so.com/s?q={encoded}");

    let Ok(resp) = client
        .get(&url)
        .header("User-Agent", UA)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .await
    else {
        return hits;
    };

    let Ok(html) = resp.text().await else {
        return hits;
    };

    for block in html.split("<li class=\"res-list\"").skip(1) {
        let end = block.find("</li>").unwrap_or(block.len());
        let item = &block[..end];

        let Some(h3_start) = item.find("<h3") else { continue };
        let after_h3 = &item[h3_start..];
        let Some(h3_end) = after_h3.find("</h3>") else { continue };
        let h3_html = &after_h3[..h3_end];

        let link = extract_attr(h3_html, "data-mdurl")
            .or_else(|| extract_attr(h3_html, "href"))
            .unwrap_or_default();
        if !link.starts_with("http") {
            continue;
        }
        let title = strip_html(h3_html);
        if title.is_empty() {
            continue;
        }

        let mut snippet = String::new();
        for cls in ["res-desc", "res-rich", "summary"] {
            if let Some(idx) = item.find(cls) {
                let sub = &item[idx..];
                if let Some(gt) = sub.find('>') {
                    let content = &sub[gt + 1..];
                    let close = content.find("</p>").or_else(|| content.find("</div>")).unwrap_or(content.len().min(500));
                    let s = strip_html(&content[..close]);
                    if s.chars().count() > 10 {
                        snippet = s.chars().take(220).collect();
                        break;
                    }
                }
            }
        }
        if snippet.is_empty() {
            continue;
        }

        hits.push(SearchHit {
            title,
            url: link,
            snippet,
            source: "360搜索".into(),
        });
        if hits.len() >= max_results {
            break;
        }
    }

    hits
}

/// 引擎 3：DuckDuckGo HTML 版（国际网络免 Key 备用）
async fn search_ddg(client: &reqwest::Client, query: &str, max_results: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let encoded = url_encode(query);
    let url = format!("https://html.duckduckgo.com/html/?q={encoded}");

    let Ok(resp) = client
        .get(&url)
        .header("User-Agent", UA)
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .send()
        .await
    else {
        return hits;
    };

    let Ok(html) = resp.text().await else {
        return hits;
    };

    for block in html.split("class=\"result__body\"").skip(1) {
        let end = block.find("<div class=\"clear\"></div>").unwrap_or(block.len().min(2000));
        let item = &block[..end];

        let Some(a_idx) = item.find("class=\"result__a\"") else { continue };
        let before_a = &item[..a_idx + 20];
        let tag_start = before_a.rfind("<a ").unwrap_or(0);
        let a_sub = &item[tag_start..];
        let Some(a_close) = a_sub.find("</a>") else { continue };
        let a_full = &a_sub[..a_close];

        let raw_href = extract_attr(a_full, "href").unwrap_or_default();
        let link = if let Some(u_idx) = raw_href.find("uddg=") {
            let sub = &raw_href[u_idx + 5..];
            let amp = sub.find('&').unwrap_or(sub.len());
            url_decode(&sub[..amp])
        } else {
            raw_href
        };
        if !link.starts_with("http") {
            continue;
        }
        let title = strip_html(a_full);
        if title.is_empty() {
            continue;
        }

        let mut snippet = String::new();
        if let Some(sn_idx) = item.find("result__snippet") {
            let sub = &item[sn_idx..];
            if let Some(gt) = sub.find('>') {
                let content = &sub[gt + 1..];
                let close = content.find("</a>").or_else(|| content.find("</div>")).unwrap_or(content.len().min(500));
                snippet = strip_html(&content[..close]).chars().take(220).collect();
            }
        }
        if snippet.is_empty() {
            continue;
        }

        hits.push(SearchHit {
            title,
            url: link,
            snippet,
            source: "DuckDuckGo".into(),
        });
        if hits.len() >= max_results {
            break;
        }
    }

    hits
}

/// 执行免费联网搜索（自动多源聚合：Bing -> 360 -> DuckDuckGo）
pub async fn search(raw_query: &str, max_results: usize) -> SearchResponse {
    let query = clean_query(raw_query);
    let date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let limit = max_results.clamp(1, 8);

    if query.is_empty() {
        return SearchResponse {
            query,
            date,
            hits: vec![],
        };
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_default();

    // 1. 优先使用国内直连最快的 Bing 搜索
    let mut hits = search_bing(&client, &query, limit).await;

    // 2. 若结果不足，使用 360 搜索补充
    if hits.len() < 2 {
        let mut so_hits = search_so360(&client, &query, limit).await;
        for h in so_hits.drain(..) {
            if !hits.iter().any(|x| x.title == h.title || x.url == h.url) {
                hits.push(h);
                if hits.len() >= limit {
                    break;
                }
            }
        }
    }

    // 3. 若仍为空，回退至 DuckDuckGo HTML
    if hits.is_empty() {
        hits = search_ddg(&client, &query, limit).await;
    }

    SearchResponse { query, date, hits }
}
