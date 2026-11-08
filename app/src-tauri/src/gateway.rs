//! 对外 OpenAI 兼容网关：其他应用通过 http://127.0.0.1:18181/v1 调用模型与本地视觉服务。
//! 所有请求都经过模型管家（按需加载 / 闲置计时 / 显卡让道），llama-server 只对内暴露。
use crate::llm::LlmError;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Clone)]
struct Gw {
    app: AppHandle,
}

pub const MODEL_ID: &str = "minibot-2b";

fn err(status: StatusCode, kind: &str, msg: &str, extra: Value) -> Response {
    let mut e = json!({ "error": { "message": msg, "type": kind } });
    if let (Some(o), Some(x)) = (e["error"].as_object_mut(), extra.as_object()) {
        for (k, v) in x {
            o.insert(k.clone(), v.clone());
        }
    }
    (status, Json(e)).into_response()
}

fn check_auth(gw: &Gw, headers: &HeaderMap) -> Result<(), Response> {
    let key = gw.app.state::<crate::AppState>().settings.get().api.api_key;
    if key.is_empty() {
        return Ok(());
    }
    let ok = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_start_matches("Bearer ").trim() == key)
        .unwrap_or(false);
    if ok { Ok(()) } else { Err(err(StatusCode::UNAUTHORIZED, "invalid_api_key", "API Key 无效", Value::Null)) }
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let st = app.state::<crate::AppState>();
        let cfg = st.settings.get().api;
        if !cfg.enabled {
            return;
        }
        // 局域网访问必须配置 API Key，否则只监听本机
        let host = if cfg.lan && !cfg.api_key.is_empty() { "0.0.0.0" } else { "127.0.0.1" };
        let listener = match tokio::net::TcpListener::bind((host, cfg.port)).await {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("OpenAI 接口端口 {} 被占用或无法监听：{e}", cfg.port);
                *st.gateway_error.lock().unwrap() = Some(msg);
                return;
            }
        };
        *st.gateway_error.lock().unwrap() = None;
        let gw = Gw { app: app.clone() };
        let router = Router::new()
            .route("/health", get(health))
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat))
            .route("/v1/completions", post(completions))
            .route("/v1/ocr", post(ocr))
            .route("/v1/qr/decode", post(qr_decode))
            .route("/v1/qr/encode", post(qr_encode))
            .fallback(fallback)
            .layer(axum::middleware::map_response(cors))
            .with_state(gw);
        let _ = axum::serve(listener, router).await;
    });
}

async fn cors(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    h.insert("access-control-allow-headers", HeaderValue::from_static("authorization, content-type"));
    h.insert("access-control-allow-methods", HeaderValue::from_static("GET, POST, OPTIONS"));
    r
}

async fn fallback(method: Method) -> Response {
    if method == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        err(StatusCode::NOT_FOUND, "not_found", "接口不存在", Value::Null)
    }
}

async fn health(State(gw): State<Gw>) -> Json<Value> {
    let st = gw.app.state::<crate::AppState>();
    let info = st.llm.status_info(&st.settings.get().llm);
    Json(json!({ "status": "ok", "model": MODEL_ID, "llm": info }))
}

async fn models(State(gw): State<Gw>, headers: HeaderMap) -> Response {
    if let Err(r) = check_auth(&gw, &headers) {
        return r;
    }
    Json(json!({
        "object": "list",
        "data": [{ "id": MODEL_ID, "object": "model", "created": 0, "owned_by": "minibot", "capabilities": ["text", "vision"] }]
    }))
    .into_response()
}

/// llama-server 不一定能拉取远程图片：把 http(s) 图片下载后转为 data URI
async fn inline_remote_images(body: &mut Value) {
    let Some(msgs) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else { return };
    let client = reqwest::Client::new();
    for m in msgs {
        let Some(parts) = m.get_mut("content").and_then(|c| c.as_array_mut()) else { continue };
        for p in parts {
            let Some(url) = p.pointer("/image_url/url").and_then(|u| u.as_str()).map(String::from) else { continue };
            if !url.starts_with("http") {
                continue;
            }
            if let Ok(resp) = client.get(&url).send().await {
                let mime = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/png")
                    .to_string();
                if let Ok(bytes) = resp.bytes().await {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    p["image_url"]["url"] = Value::String(format!("data:{mime};base64,{b64}"));
                }
            }
        }
    }
}

async fn chat(State(gw): State<Gw>, headers: HeaderMap, Json(mut body): Json<Value>) -> Response {
    if let Err(r) = check_auth(&gw, &headers) {
        return r;
    }
    inline_remote_images(&mut body).await;
    proxy(gw, "/v1/chat/completions", body).await
}

async fn completions(State(gw): State<Gw>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = check_auth(&gw, &headers) {
        return r;
    }
    proxy(gw, "/v1/completions", body).await
}

async fn proxy(gw: Gw, path: &str, mut body: Value) -> Response {
    let st = gw.app.state::<crate::AppState>();
    let settings = st.settings.get();

    // 若已启用云端 API 模型，网关请求直接代理至云端 OpenAI 兼容接口
    if settings.llm.api_enabled && path == "/v1/chat/completions" {
        if let Some(o) = body.as_object_mut() {
            o.insert("model".into(), Value::String(settings.llm.api_model.clone()));
        }
        let endpoint = crate::llm::resolve_chat_completions_url(&settings.llm.api_base_url);
        let client = match crate::llm::build_http_client(&endpoint, 120) {
            Ok(c) => c,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &e.to_string(), Value::Null),
        };
        let mut req = client.post(&endpoint).header("content-type", "application/json");
        if !settings.llm.api_key.trim().is_empty() {
            req = req.header("authorization", format!("Bearer {}", settings.llm.api_key.trim()));
        }
        let resp = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => return err(StatusCode::BAD_GATEWAY, "upstream_error", &e.to_string(), Value::Null),
        };
        let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();
        let stream = resp.bytes_stream().map(|r| r.map_err(std::io::Error::other));
        return Response::builder()
            .status(status)
            .header("content-type", ct)
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    if let Some(o) = body.as_object_mut() {
        o.remove("model");
    }
    // 调用方未指定时，按设置决定是否启用思考链（默认关闭以提速）
    if let Some(o) = body.as_object_mut() {
        if !o.contains_key("chat_template_kwargs") {
            o.insert("chat_template_kwargs".into(), json!({ "enable_thinking": settings.api.thinking }));
        }
    }
    let force = settings.api.busy_policy == "force";
    let guard = match st.llm.acquire(&gw.app, force).await {
        Ok(g) => g,
        Err(LlmError::GpuBusy { snapshot, required_mb }) => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                "gpu_busy",
                &format!(
                    "显卡正忙（已用 {}MB / {}MB，剩余 {}MB，需要约 {}MB），请稍后再试",
                    snapshot.used_mb, snapshot.total_mb, snapshot.free_mb, required_mb
                ),
                json!({ "processes": snapshot.processes }),
            );
        }
        Err(LlmError::PortBusy { port }) => {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "port_busy", &format!("内部推理端口 {port} 被占用"), Value::Null);
        }
        Err(e) => {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &format!("{e:?}"), Value::Null);
        }
    };
    let url = format!("http://127.0.0.1:{}{}", settings.llm.port, path);
    let client = match reqwest::Client::builder().no_proxy().build() {
        Ok(c) => c,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &e.to_string(), Value::Null),
    };
    let resp = match client.post(url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, "upstream_error", &e.to_string(), Value::Null),
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    // guard 随流一起释放：流结束前模型不会被闲置回收
    let stream = resp.bytes_stream().map(move |r| {
        let _keep = &guard;
        r.map_err(std::io::Error::other)
    });
    Response::builder()
        .status(status)
        .header("content-type", ct)
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn image_from(body: &Value) -> Result<image::RgbaImage, Response> {
    let s = body.get("image").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = crate::imgutil::decode_b64(s)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid_image", "image 需为 base64 或 data URI", Value::Null))?;
    crate::imgutil::decode_rgba(&bytes).map_err(|_| err(StatusCode::BAD_REQUEST, "invalid_image", "无法解码图片", Value::Null))
}

async fn ocr(State(gw): State<Gw>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = check_auth(&gw, &headers) {
        return r;
    }
    let img = match image_from(&body) {
        Ok(i) => i,
        Err(r) => return r,
    };
    match tokio::task::spawn_blocking(move || crate::ocr::recognize(&img)).await {
        Ok(Ok(r)) => Json(r).into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, "ocr_error", &e.to_string(), Value::Null),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "ocr_error", &e.to_string(), Value::Null),
    }
}

async fn qr_decode(State(gw): State<Gw>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = check_auth(&gw, &headers) {
        return r;
    }
    let img = match image_from(&body) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let codes = tokio::task::spawn_blocking(move || crate::qr::decode_rgba(&img)).await.unwrap_or_default();
    Json(json!({ "results": codes })).into_response()
}

async fn qr_encode(State(gw): State<Gw>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = check_auth(&gw, &headers) {
        return r;
    }
    let s = |k: &str, d: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    match crate::qr::encode_svg(&s("text", ""), &s("ec", "M"), &s("dark", "#000000"), &s("light", "#ffffff")) {
        Ok(svg) => Json(json!({ "svg": svg })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, "qr_error", &e.to_string(), Value::Null),
    }
}
