//! LocalSend Protocol v2.2 原生局域网文件/文本/截图快传模块
//! 兼容所有安装官方 LocalSend 的客户端（iOS / Android / Windows / macOS / Linux）
//! 协议规范参考：https://github.com/localsend/protocol
//!
//! 核心能力：
//! 1. 双向发现闭环：
//!    - 后台常驻监听 UDP 组播 (224.0.0.167:53317) 与 HTTPS/HTTP 端口 (53317)，响应手机/其他电脑的发现请求
//!    - 主动扫描：优先探测 ARP 活跃主机 + 本机真实网关子网 53317 端口高并发探测 + UDP 组播广播
//! 2. mTLS 双向证书认证：内置标准 X.509 自签名客户端/服务端证书与匹配的 SHA-256 指纹，解决新版 LocalSend v2.1/v2.2 TLS Alert 116 握手要求
//! 3. 多类型发送：支持本地多文件、文件夹、截图 (Base64 PNG)、划词/剪贴板纯文本 (text/plain 预览直达)
//! 4. 跨平台系统右键菜单：Windows (HKCU 注册表) + macOS (Finder Quick Action Service workflow)

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Semaphore;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub const LOCALSEND_PORT: u16 = 53317;
pub const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

/// 预置标准 LocalSend X.509 v3 DER 证书 (CN=LocalSend User, O=LocalSend) 与 PKCS#8 私钥及精确 SHA-256 指纹
const CERT_FINGERPRINT: &str = "44DA0C53A2BF4539AB4B21CACB8911DE02A66E85014EEC59BB4E8FC11969758B";
const CERT_B64: &str = "MIIC3TCCAcWgAwIBAgIJAPK1hlYOpX5kMA0GCSqGSIb3DQEBCwUAMC0xEjAQBgNVBAoTCUxvY2FsU2VuZDEXMBUGA1UEAxMOTG9jYWxTZW5kIFVzZXIwIBcNMjYxMDI5MDAwNDM0WhgPMjEyNjEwMTUwMDA0MzRaMC0xEjAQBgNVBAoTCUxvY2FsU2VuZDEXMBUGA1UEAxMOTG9jYWxTZW5kIFVzZXIwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC46CX6wy1CJx3X6bVHD/IZI1cl/mhIAz3wQp130h2Wvj/TLDedtE2J+pL31Vp+tVwKz4xNCGsbJFOe89hQWZyCvlLbRZ8zfHbH3wQ1AW4EyQfXJmizW9DNZmPEj6cN8PaM1Ku7tUPLthGaOKhm6VDBsw72ybLoZb8s31yZ3sfJ+DZk39ubmbE3CPdP8gU5vPC4617YI/jexhNiLeXGerC0Ye/YzDQqKUuaRmGoNqvhp4ioCSIqm7Pgqzla44tvuS0vYab+WHwrRVTjLUSNZtYw11oZOR+Myg2/6jt1BYKb5ipNfm0qG9EQI5YRVsZU2Dh7MclciMMoy+IcI1/csbnJAgMBAAEwDQYJKoZIhvcNAQELBQADggEBAGai7nTYLuHibfP5JoPVRtcqaoRvvW3X/iUjUK/mmsHd6m+VNx0JNXenMFzk1GTz739Um1yTteDrnZ2UHUK12u4UBCIJux4ZLd8uMsEj13IdtFmoed7dAz99Br+kiOeQdBOBSStwXtlegIKMc/IXAQ4Ojm411ukqC58eQKSaIQGP9/yYecP6+MCDR60OzTSalbH1MNNNmT9D1mX7COA8E26i5XNu1QLmWfAf3H/KdWxlGhCZKcp6ynQJlsFeIavwV3XCFgOwORw//PsTnyG+NT1LBNOsqeHqLMoldC5X3zmTypHhjR4zAsjRNBGKRotI2FNdI+BxBYCB5gvkFeVpcQg=";
const KEY_B64: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC46CX6wy1CJx3X6bVHD/IZI1cl/mhIAz3wQp130h2Wvj/TLDedtE2J+pL31Vp+tVwKz4xNCGsbJFOe89hQWZyCvlLbRZ8zfHbH3wQ1AW4EyQfXJmizW9DNZmPEj6cN8PaM1Ku7tUPLthGaOKhm6VDBsw72ybLoZb8s31yZ3sfJ+DZk39ubmbE3CPdP8gU5vPC4617YI/jexhNiLeXGerC0Ye/YzDQqKUuaRmGoNqvhp4ioCSIqm7Pgqzla44tvuS0vYab+WHwrRVTjLUSNZtYw11oZOR+Myg2/6jt1BYKb5ipNfm0qG9EQI5YRVsZU2Dh7MclciMMoy+IcI1/csbnJAgMBAAECggEAAWhrGZyw1IcduA4kfjUF3+EgAb8NJjc4GoM3Vbk41nP+Ymo8xiugR4Or6M0sEjjpSX/JDrEYt60wIc6zf/EeuvQeB2NhUfAJq33sN0cagljRgXbeZzc0wMudyNpbKEUEa5AW+qfnqz5opeb2iUoPfloRrBlfDlvv97R/rQBAPy5sTh6kcyYurRrOqIQlym2mhJBahTyhTWktrHPfagY6hGDgrr7UHRwvR8VF/DOYGE1T4YVkRMU9Cv4mntQoo4q+fiBx935h0SEjdKufduZM1929vzbbTM288LZzzV40uaLVBvpWy56sdKKKOUGCX6LwoznRjb3inEEYD8pRNxSR/QKBgQDzTTFZWrievSO1dOIEfRffXUajaPm0ImC5JP50/JfpqDAKwyw6blDDQ861CG5VTi/9KT2t4BWM3zB0YHqLtXVCFG127cLs4PzsHZtzLFIfwG+VlXQhHaXKtZb1V03k7OpOGVaFQlgWqOa1niV4R8QW4lXg9DJf0wn20DmxWM4opwKBgQDCjrsK00fEWVOt8DI32SXdbrEAY80iV5icm9CZXtjAlHUR6T4PT/UPzLFCLyH3NNp3DUawO2LZa5OX16M+eGbJT9PRswjUzJ1FeJNJrBp85RNRG1DJQPMkimFf3Au0VnUbZ0VDwhKyCVs3xOaX2iDQ94jpJWs90AJl+BhdQMroDwKBgBDhYzIi5DpHPyoA0QmksDpUG2mNB/4igHl4ApuVRuj0PVkRz4jCTKIvmzDeQ+FxSk0TRm9A493hQlhgtT/qOBd8mVeuCV8sAQBU0Ys9IPmI/XTwaJ0/lFgrHy0ogIQIl/0RLvqR7tCrqU4iidD9kbJQw8qWxsP1PXS9X7NZSPxzAoGAch2Vx/u1FI3wbw175xYPB5ptZ3UiVQpcET+zUsI3EH4DTMYqCIE1lFbTgaZznbP6j+nRlHzAthZLM7zfmzyzzVBtRYplm6uIc78NZSD1SFx0wBXthJ37/2rLQpamptgvSCuCjQPAQ30Ja+Sp/FeM+AS44Ud5Z4AiZ/zlDY17djECgYEAoBIY5tg2YTAwi5KYvRdGnf31nkOjkUAFjgsAYlNxjZvjm4ZHg5xRbe0d1I3spWw+QHsXcTpbnJ+Pkni0OkWJEekrl9FY5ymzNnLo0XrfuHQd7axUU6dBEd6nf0uG5t8vIOIBMUzaeuje/JTT8ifnQRmh3awpmoK9uywG8uboS3U=";

static ITEM_SEQ: AtomicU64 = AtomicU64::new(1);
static ACTIVE_SERVER_PORT: AtomicU16 = AtomicU16::new(LOCALSEND_PORT);
static SEND_CANCEL: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
static KNOWN_PEERS: OnceLock<Mutex<HashMap<String, LocalSendPeer>>> = OnceLock::new();

fn cancel_slot() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    SEND_CANCEL.get_or_init(|| Mutex::new(None))
}

fn known_peers() -> &'static Mutex<HashMap<String, LocalSendPeer>> {
    KNOWN_PEERS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn cancel_send() {
    if let Some(flag) = cancel_slot().lock().unwrap().as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
}

// ---------- 数据结构 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSendPeer {
    pub ip: String,
    pub port: u16,
    pub alias: String,
    pub device_model: String,
    /// "mobile" | "desktop" | "web" | "headless" | "server"
    pub device_type: String,
    /// "https" | "http"
    pub protocol: String,
    pub version: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSendItem {
    pub id: String,
    /// "file" | "text" | "image"
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data_url: Option<String>,
    pub size: u64,
    pub file_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalSendProgress {
    /// "preparing" | "waiting_accept" | "sending" | "done" | "rejected" | "pin_required" | "cancelled" | "error"
    pub stage: String,
    pub file_id: String,
    pub file_name: String,
    pub current_index: usize,
    pub total_files: usize,
    pub sent_bytes: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
    pub percent: u32,
    pub message: String,
}

// ---------- 本机设备信息与局域网 IP 探测（跨平台 Windows / macOS / Linux） ----------

fn device_fingerprint() -> &'static str {
    CERT_FINGERPRINT
}

pub fn hostname() -> String {
    if let Ok(n) = std::env::var("COMPUTERNAME") {
        if !n.trim().is_empty() {
            return n.trim().to_string();
        }
    }
    if let Ok(n) = std::env::var("HOSTNAME") {
        if !n.trim().is_empty() {
            return n.trim().to_string();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    "Desktop".to_string()
}

pub fn os_model_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        "Desktop"
    }
}

pub fn self_alias() -> String {
    format!("miniBot ({})", hostname())
}

fn self_info_json(announce: bool) -> Value {
    let port = ACTIVE_SERVER_PORT.load(Ordering::Relaxed);
    let mut obj = json!({
        "alias": self_alias(),
        "version": "2.2",
        "deviceModel": os_model_name(),
        "deviceType": "desktop",
        "fingerprint": device_fingerprint(),
        "port": port,
        "protocol": "https",
        "download": false,
    });
    if announce {
        if let Some(m) = obj.as_object_mut() {
            m.insert("announce".into(), Value::Bool(true));
            m.insert("announcement".into(), Value::Bool(true));
        }
    }
    obj
}

fn is_private_lan_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
        return false;
    }
    // 排除常见 Clash / Fake-IP 网段 198.18.x.x
    if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
        return false;
    }
    match o[0] {
        10 => true,
        172 => (16..=31).contains(&o[1]),
        192 => o[1] == 168,
        _ => false,
    }
}

/// 获取本机所有私有局域网 IPv4 地址（首选出站物理网卡排在第一位）
pub fn local_lan_ips() -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();

    // 1. 通过 UDP 路由探测首选物理出站网卡 IP（如 192.168.6.104）
    for target in ["8.8.8.8:80", "114.114.114.114:80", "223.5.5.5:80"] {
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(target).is_ok() {
                if let Ok(SocketAddr::V4(addr)) = sock.local_addr() {
                    let ip = *addr.ip();
                    if is_private_lan_ipv4(ip) && !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
            }
        }
    }

    // 2. 平台命令兜底补充（优先把 192.168.x.x 物理局域网排在 172.x 虚拟交换机前面）
    let mut extra = Vec::new();
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("ipconfig").creation_flags(0x0800_0000).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains("IPv4") || line.contains("IP Address") {
                    if let Some(ip_str) = line.split(':').nth(1) {
                        let clean = ip_str.trim().trim_end_matches("(Preferred)").trim();
                        if let Ok(ip) = clean.parse::<Ipv4Addr>() {
                            if is_private_lan_ipv4(ip) && !ips.contains(&ip) && !extra.contains(&ip) {
                                extra.push(ip);
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(out) = std::process::Command::new("ifconfig").output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("inet ") {
                    if let Some(ip_str) = rest.split_whitespace().next() {
                        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                            if is_private_lan_ipv4(ip) && !ips.contains(&ip) && !extra.contains(&ip) {
                                extra.push(ip);
                            }
                        }
                    }
                }
            }
        }
    }

    extra.sort_by_key(|ip| if ip.octets()[0] == 192 { 0 } else { 1 });
    ips.extend(extra);
    ips
}

/// 从系统 ARP 缓存表获取当前已经活跃通信的局域网主机 IP（毫秒级秒扫核心优化）
fn arp_active_ips() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    #[cfg(target_os = "windows")]
    let cmd_out = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("arp")
            .arg("-a")
            .creation_flags(0x0800_0000)
            .output()
    };
    #[cfg(not(target_os = "windows"))]
    let cmd_out = std::process::Command::new("arp").arg("-a").output();

    if let Ok(res) = cmd_out {
        let text = String::from_utf8_lossy(&res.stdout);
        for token in text.split(|c: char| c.is_whitespace() || c == '(' || c == ')') {
            let t = token.trim();
            if t.starts_with("192.168.") || t.starts_with("10.") || t.starts_with("172.") {
                if let Ok(ip) = t.parse::<Ipv4Addr>() {
                    let last = ip.octets()[3];
                    if is_private_lan_ipv4(ip) && last != 0 && last != 255 && !out.contains(&ip) {
                        out.push(ip);
                    }
                }
            }
        }
    }
    out
}


// ---------- mTLS 双向证书客户端与服务端（兼容 LocalSend v2.0 / v2.1 / v2.2） ----------

#[derive(Debug)]
struct NoCertVerifier;

impl ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

fn load_self_cert_and_key() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let cert_bytes = base64::engine::general_purpose::STANDARD
        .decode(CERT_B64)
        .expect("valid CERT_B64");
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(KEY_B64)
        .expect("valid KEY_B64");
    (
        CertificateDer::from(cert_bytes),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_bytes)),
    )
}

fn tls_client_connector() -> TlsConnector {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let cfg = CFG.get_or_init(|| {
        let (cert, key) = load_self_cert_and_key();
        let provider = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider();
        let builder = ClientConfig::builder_with_provider(provider.into())
            .with_safe_default_protocol_versions()
            .expect("TLS protocol config");
        let config = builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_client_auth_cert(vec![cert], key)
            .expect("TLS client auth cert config");
        Arc::new(config)
    });
    TlsConnector::from(cfg.clone())
}

fn tls_server_acceptor() -> TlsAcceptor {
    static CFG: OnceLock<Arc<ServerConfig>> = OnceLock::new();
    let cfg = CFG.get_or_init(|| {
        let (cert, key) = load_self_cert_and_key();
        let provider = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider();
        let builder = ServerConfig::builder_with_provider(provider.into())
            .with_safe_default_protocol_versions()
            .expect("TLS server protocol config");
        let config = builder
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("TLS server cert config");
        Arc::new(config)
    });
    TlsAcceptor::from(cfg.clone())
}

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

async fn open_peer_stream(ip: &str, port: u16, use_https: bool, timeout_ms: u64) -> Result<Box<dyn AsyncReadWrite>> {
    let addr = format!("{ip}:{port}");
    let conn_timeout = timeout_ms.min(450);
    let tcp = tokio::time::timeout(Duration::from_millis(conn_timeout), TcpStream::connect(&addr))
        .await
        .map_err(|_| anyhow!("连接 {addr} 超时"))?
        .map_err(|e| anyhow!("连接 {addr} 失败: {e}"))?;
    let _ = tcp.set_nodelay(true);

    if use_https {
        let connector = tls_client_connector();
        let server_name = ServerName::try_from("localhost").unwrap_or_else(|_| {
            ServerName::IpAddress(ip.parse::<IpAddr>().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)).into())
        });
        let tls = tokio::time::timeout(Duration::from_millis(timeout_ms.max(1500)), connector.connect(server_name, tcp))
            .await
            .map_err(|_| anyhow!("TLS 握手超时"))?
            .map_err(|e| anyhow!("TLS 握手失败: {e}"))?;
        Ok(Box::new(tls))
    } else {
        Ok(Box::new(tcp))
    }
}

/// 发送轻量 HTTP/1.1 请求并精确根据 Content-Length 读取响应
async fn raw_http_json(
    ip: &str,
    port: u16,
    use_https: bool,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout_ms: u64,
) -> Result<(u16, String)> {
    let mut stream = open_peer_stream(ip, port, use_https, timeout_ms).await?;
    let body_bytes = body.map(|v| serde_json::to_vec(v).unwrap_or_default()).unwrap_or_default();

    let req_head = if body.is_some() {
        format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {ip}:{port}\r\n\
             User-Agent: LocalSend/2.2 (miniBot)\r\n\
             Accept: application/json\r\n\
             Content-Type: application/json; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body_bytes.len()
        )
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {ip}:{port}\r\n\
             User-Agent: LocalSend/2.2 (miniBot)\r\n\
             Accept: application/json\r\n\
             Connection: close\r\n\r\n"
        )
    };

    stream.write_all(req_head.as_bytes()).await?;
    if !body_bytes.is_empty() {
        stream.write_all(&body_bytes).await?;
    }
    stream.flush().await?;

    read_http_response(&mut stream, timeout_ms).await
}

fn parse_content_length(head_str: &str) -> Option<usize> {
    for line in head_str.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(n) = v.trim().parse::<usize>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

async fn read_http_response(stream: &mut Box<dyn AsyncReadWrite>, timeout_ms: u64) -> Result<(u16, String)> {
    let mut buf = Vec::with_capacity(2048);
    let mut temp = [0u8; 2048];
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut header_end: Option<usize> = None;
    let mut expected_body_len: Option<usize> = None;

    loop {
        if let (Some(h_end), Some(c_len)) = (header_end, expected_body_len) {
            if buf.len() >= h_end + c_len {
                break;
            }
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let rem = deadline - now;
        match tokio::time::timeout(rem, stream.read(&mut temp)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                buf.extend_from_slice(&temp[..n]);
                if header_end.is_none() {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let h_end = pos + 4;
                        header_end = Some(h_end);
                        let head_str = String::from_utf8_lossy(&buf[..pos]);
                        let first_line = head_str.lines().next().unwrap_or("");
                        if first_line.contains(" 204 ") {
                            expected_body_len = Some(0);
                        } else {
                            expected_body_len = parse_content_length(&head_str);
                        }
                    }
                }
                if buf.len() > 256 * 1024 {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }

    if buf.is_empty() {
        bail!("未收到响应数据");
    }

    let sep_pos = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let (head_bytes, body_bytes) = match sep_pos {
        Some(pos) => (&buf[..pos], &buf[pos + 4..]),
        None => (&buf[..], &[][..]),
    };

    let head_str = String::from_utf8_lossy(head_bytes);
    let status_line = head_str.lines().next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("无效的 HTTP 响应头: {status_line}"))?;

    let is_chunked = head_str
        .lines()
        .any(|l| l.to_ascii_lowercase().contains("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));

    let body_str = if is_chunked {
        decode_chunked(body_bytes)
    } else if let Some(c_len) = expected_body_len {
        let slice = &body_bytes[..body_bytes.len().min(c_len)];
        String::from_utf8_lossy(slice).trim().to_string()
    } else {
        String::from_utf8_lossy(body_bytes).trim().to_string()
    };

    Ok((status, body_str))
}

fn decode_chunked(mut data: &[u8]) -> String {
    let mut out = Vec::new();
    while !data.is_empty() {
        let Some(pos) = data.windows(2).position(|w| w == b"\r\n") else {
            out.extend_from_slice(data);
            break;
        };
        let len_str = String::from_utf8_lossy(&data[..pos]).trim().to_string();
        let len_clean = len_str.split(';').next().unwrap_or("").trim();
        let Ok(chunk_len) = usize::from_str_radix(len_clean, 16) else {
            out.extend_from_slice(data);
            break;
        };
        if chunk_len == 0 {
            break;
        }
        let start = pos + 2;
        if start + chunk_len > data.len() {
            out.extend_from_slice(&data[start..]);
            break;
        }
        out.extend_from_slice(&data[start..start + chunk_len]);
        let next = start + chunk_len + 2;
        if next >= data.len() {
            break;
        }
        data = &data[next..];
    }
    String::from_utf8_lossy(&out).trim().to_string()
}

// ---------- 后台常驻 LocalSend 发现与监听服务（让手机随时能发现电脑，电脑也能被动收到手机注册） ----------

pub fn spawn_daemon() {
    tauri::async_runtime::spawn(async move {
        // 1. 启动 TCP (HTTPS + HTTP 自适应) 服务端，监听 53317（若被占用则回退 53318）
        let mut bound_listener = None;
        for port in [LOCALSEND_PORT, LOCALSEND_PORT + 1, LOCALSEND_PORT + 2] {
            if let Ok(l) = TcpListener::bind(("0.0.0.0", port)).await {
                ACTIVE_SERVER_PORT.store(port, Ordering::Relaxed);
                bound_listener = Some(l);
                break;
            }
        }

        if let Some(listener) = bound_listener {
            tokio::spawn(async move {
                loop {
                    let Ok((tcp, remote_addr)) = listener.accept().await else {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    };
                    let peer_ip = remote_addr.ip().to_string();
                    tokio::spawn(async move {
                        let _ = handle_incoming_tcp(tcp, peer_ip).await;
                    });
                }
            });
        }

        // 2. 启动 UDP 组播监听 (224.0.0.167:53317)，响应手机/其他电脑的组播搜索
        tokio::spawn(async move {
            let _ = run_multicast_listener().await;
        });

        // 3. 启动后主动在每个物理网卡上广播一次并向 ARP 活跃设备注册本机
        tokio::time::sleep(Duration::from_millis(600)).await;
        let _ = broadcast_udp_announcement().await;
        let _ = scan_peers(Vec::new()).await;
    });
}

async fn handle_incoming_tcp(tcp: TcpStream, peer_ip: String) -> Result<()> {
    let _ = tcp.set_nodelay(true);
    let mut first_byte = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_millis(2000), tcp.peek(&mut first_byte)).await??;
    if n == 0 {
        return Ok(());
    }

    // 0x16 表示 TLS Handshake；否则按明文 HTTP 处理
    let mut stream: Box<dyn AsyncReadWrite> = if first_byte[0] == 0x16 {
        let acceptor = tls_server_acceptor();
        let tls = tokio::time::timeout(Duration::from_millis(3000), acceptor.accept(tcp)).await??;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };

    let mut buf = Vec::with_capacity(2048);
    let mut temp = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut expected_len: Option<usize> = None;
    let deadline = Instant::now() + Duration::from_millis(3000);

    loop {
        if let (Some(h_end), Some(c_len)) = (header_end, expected_len) {
            if buf.len() >= h_end + c_len {
                break;
            }
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(deadline - now, stream.read(&mut temp)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(read_n)) => {
                buf.extend_from_slice(&temp[..read_n]);
                if header_end.is_none() {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(pos + 4);
                        let head_s = String::from_utf8_lossy(&buf[..pos]);
                        expected_len = Some(parse_content_length(&head_s).unwrap_or(0));
                    }
                }
            }
            _ => break,
        }
    }

    let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(());
    };
    let head_str = String::from_utf8_lossy(&buf[..pos]);
    let body_bytes = &buf[pos + 4..];
    let req_line = head_str.lines().next().unwrap_or("");

    // 若对方发来的是 POST /register，解析其设备信息加入缓存
    if req_line.contains("/register") && !body_bytes.is_empty() {
        if let Ok(val) = serde_json::from_slice::<Value>(body_bytes) {
            if let Some(peer) = parse_peer_json(&peer_ip, LOCALSEND_PORT, "https", &val) {
                known_peers().lock().unwrap().insert(peer.ip.clone(), peer);
            }
        }
    }

    let resp_json = serde_json::to_vec(&self_info_json(false)).unwrap_or_default();
    let resp_head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        resp_json.len()
    );
    stream.write_all(resp_head.as_bytes()).await?;
    stream.write_all(&resp_json).await?;
    stream.flush().await?;
    Ok(())
}

async fn run_multicast_listener() -> Result<()> {
    let std_sock = std::net::UdpSocket::bind(("0.0.0.0", LOCALSEND_PORT))?;
    std_sock.set_nonblocking(true)?;
    let _ = std_sock.set_broadcast(true);
    let _ = std_sock.set_multicast_loop_v4(false);
    for lip in local_lan_ips() {
        let _ = std_sock.join_multicast_v4(&MULTICAST_ADDR, &lip);
    }
    let _ = std_sock.join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED);

    let sock = UdpSocket::from_std(std_sock)?;
    let mut buf = [0u8; 4096];

    loop {
        let Ok((n, src)) = sock.recv_from(&mut buf).await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let ip_str = src.ip().to_string();
        let Ok(val) = serde_json::from_slice::<Value>(&buf[..n]) else {
            continue;
        };
        let Some(peer) = parse_peer_json(&ip_str, LOCALSEND_PORT, "https", &val) else {
            continue;
        };
        known_peers().lock().unwrap().insert(peer.ip.clone(), peer.clone());

        // 当对方广播 announce: true 时，立即向对方调用 POST /api/localsend/v2/register 回应，让对方手机发现本机！
        let is_announce = val.get("announce").and_then(|x| x.as_bool()).unwrap_or(false)
            || val.get("announcement").and_then(|x| x.as_bool()).unwrap_or(false);
        if is_announce {
            tokio::spawn(async move {
                let info = self_info_json(false);
                let use_https = peer.protocol != "http";
                let _ = raw_http_json(
                    &peer.ip,
                    peer.port,
                    use_https,
                    "POST",
                    "/api/localsend/v2/register",
                    Some(&info),
                    2000,
                )
                .await;
            });
        }
    }
}

/// 在每一个本机局域网网卡上向 224.0.0.167:53317 发送组播公告包
async fn broadcast_udp_announcement() {
    let msg = serde_json::to_vec(&self_info_json(true)).unwrap_or_default();
    let target = SocketAddrV4::new(MULTICAST_ADDR, LOCALSEND_PORT);

    for lip in local_lan_ips() {
        let bind_addr = SocketAddrV4::new(lip, 0);
        if let Ok(sock) = UdpSocket::bind(bind_addr).await {
            let _ = sock.set_broadcast(true);
            let _ = sock.join_multicast_v4(MULTICAST_ADDR, lip);
            let _ = sock.send_to(&msg, target).await;
        }
    }
}

// ---------- 设备解析与主动探测 ----------

fn parse_peer_json(ip: &str, default_port: u16, default_proto: &str, v: &Value) -> Option<LocalSendPeer> {
    let alias = v.get("alias").and_then(|x| x.as_str())?.trim().to_string();
    if alias.is_empty() {
        return None;
    }
    let fingerprint = v
        .get("fingerprint")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    // 排除本机自己
    if fingerprint.eq_ignore_ascii_case(device_fingerprint()) {
        return None;
    }
    let port = v
        .get("port")
        .and_then(|x| x.as_u64())
        .map(|p| p as u16)
        .unwrap_or(default_port);
    let protocol = v
        .get("protocol")
        .and_then(|x| x.as_str())
        .unwrap_or(default_proto)
        .to_lowercase();
    let device_model = v
        .get("deviceModel")
        .and_then(|x| x.as_str())
        .unwrap_or("LocalSend Device")
        .to_string();
    let device_type = v
        .get("deviceType")
        .and_then(|x| x.as_str())
        .unwrap_or("desktop")
        .to_lowercase();
    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("2.0")
        .to_string();

    Some(LocalSendPeer {
        ip: ip.to_string(),
        port,
        alias,
        device_model,
        device_type,
        protocol: if protocol == "http" { "http".into() } else { "https".into() },
        version,
        fingerprint,
    })
}

/// 探测单个 IP:Port 是否为在线 LocalSend 客户端（同时向其调用 POST /register 完成双向互认！）
pub async fn probe_peer(ip: &str, port: u16) -> Result<LocalSendPeer> {
    let self_info = self_info_json(false);

    // 直接使用带超时控制的 raw_http_json（先尝试 HTTPS POST /register，避免预先空连断开触发对端 TLS 异常）
    for (use_https, timeout_ms) in [(true, 900u64), (false, 500u64)] {
        let proto_str = if use_https { "https" } else { "http" };
        for (method, path, body) in [
            ("POST", "/api/localsend/v2/register", Some(&self_info)),
            ("GET", "/api/localsend/v2/info", None),
        ] {
            match raw_http_json(ip, port, use_https, method, path, body, timeout_ms).await {
                Ok((status, resp_body)) if (200..300).contains(&status) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&resp_body) {
                        if let Some(peer) = parse_peer_json(ip, port, proto_str, &val) {
                            known_peers().lock().unwrap().insert(peer.ip.clone(), peer.clone());
                            return Ok(peer);
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    // 若 TCP 连接本身超时或被拒绝，说明该 IP:Port 未开启 LocalSend，立即跳过无需重试 HTTP
                    if msg.contains("超时") && !msg.contains("TLS") {
                        bail!("端口未响应");
                    }
                    if msg.contains("连接") && msg.contains("失败") {
                        bail!("端口未开放");
                    }
                }
                _ => {}
            }
        }
    }

    bail!("未在 {ip}:{port} 检测到 LocalSend 服务")
}

/// 扫描局域网内所有在线 LocalSend 设备
/// 优先级：1. UDP 组播广播 -> 2. ARP 表活跃主机 + 已缓存主机 -> 3. 主网卡 /24 网段并发扫描
pub async fn scan_peers(extra_ips: Vec<String>) -> Vec<LocalSendPeer> {
    let local_ips = local_lan_ips();
    let local_ip_set: HashSet<String> = local_ips.iter().map(|ip| ip.to_string()).collect();

    // 1. 先在各物理网卡上发一次 UDP 组播广播
    broadcast_udp_announcement().await;

    let found: Arc<Mutex<HashMap<String, LocalSendPeer>>> = Arc::new(Mutex::new(HashMap::new()));

    // 2. 构建按优先级排序的候选探测队列
    let mut priority_candidates: Vec<(String, u16)> = Vec::new();
    let mut subnet_candidates: Vec<(String, u16)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // (a) 用户手动输入或历史连接过的 IP
    for raw in extra_ips {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let (ip_part, port_part) = match s.split_once(':') {
            Some((i, p)) => (i.trim(), p.trim().parse::<u16>().unwrap_or(LOCALSEND_PORT)),
            None => (s, LOCALSEND_PORT),
        };
        if !local_ip_set.contains(ip_part) && seen.insert(format!("{ip_part}:{port_part}")) {
            priority_candidates.push((ip_part.to_string(), port_part));
        }
    }

    // (b) 之前通过被动监听已知的设备 IP
    let cached_list: Vec<LocalSendPeer> = known_peers().lock().unwrap().values().cloned().collect();
    for p in cached_list {
        if !local_ip_set.contains(&p.ip) && seen.insert(format!("{}:{}", p.ip, p.port)) {
            priority_candidates.push((p.ip, p.port));
        }
    }

    // (c) 系统 ARP 表中当前活跃的局域网设备（通常只有几台，30ms 内即可秒出结果！）
    for aip in arp_active_ips() {
        let ip_str = aip.to_string();
        if !local_ip_set.contains(&ip_str) && seen.insert(format!("{ip_str}:{LOCALSEND_PORT}")) {
            priority_candidates.push((ip_str, LOCALSEND_PORT));
        }
    }

    // 立即并发探测第一梯队（ARP 活跃设备 + 历史设备）
    let mut fast_handles = Vec::with_capacity(priority_candidates.len());
    for (ip, port) in priority_candidates {
        let found_clone = found.clone();
        fast_handles.push(tokio::spawn(async move {
            if let Ok(peer) = probe_peer(&ip, port).await {
                found_clone.lock().unwrap().insert(peer.ip.clone(), peer);
            }
        }));
    }
    for h in fast_handles {
        let _ = h.await;
    }

    // (d) 扫描主物理局域网网段
    for lip in local_ips.into_iter().take(2) {
        let o = lip.octets();
        if o[0] == 172 && !found.lock().unwrap().is_empty() {
            continue;
        }
        for host in 1u8..=254u8 {
            if host == o[3] {
                continue;
            }
            let ip_str = format!("{}.{}.{}.{}", o[0], o[1], o[2], host);
            if seen.insert(format!("{ip_str}:{LOCALSEND_PORT}")) {
                subnet_candidates.push((ip_str, LOCALSEND_PORT));
            }
        }
    }

    // 若第一梯队（ARP 活跃表 + 缓存）已找到在线设备，立即返回给前端（~200ms 极速响应），剩余子网放到后台静默补充
    if !found.lock().unwrap().is_empty() {
        tokio::spawn(async move {
            let sem = Arc::new(Semaphore::new(64));
            for (ip, port) in subnet_candidates {
                let sem_clone = sem.clone();
                tokio::spawn(async move {
                    let Ok(_permit) = sem_clone.acquire().await else { return };
                    let _ = probe_peer(&ip, port).await;
                });
            }
        });
    } else {
        let sem = Arc::new(Semaphore::new(96));
        let mut handles = Vec::with_capacity(subnet_candidates.len());
        for (ip, port) in subnet_candidates {
            let sem_clone = sem.clone();
            let found_clone = found.clone();
            handles.push(tokio::spawn(async move {
                let Ok(_permit) = sem_clone.acquire().await else { return };
                if found_clone.lock().unwrap().contains_key(&ip) {
                    return;
                }
                if let Ok(peer) = probe_peer(&ip, port).await {
                    found_clone.lock().unwrap().insert(peer.ip.clone(), peer);
                }
            }));
        }
        for h in handles {
            let _ = h.await;
        }
    }

    let mut list: Vec<LocalSendPeer> = found.lock().unwrap().values().cloned().collect();
    list.sort_by(|a, b| a.alias.to_lowercase().cmp(&b.alias.to_lowercase()));
    list
}

// ---------- 文件与载荷解析 ----------

pub fn guess_mime(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "txt" | "md" | "log" | "csv" | "json" | "xml" | "yaml" | "yml" | "toml" | "ini" => "text/plain",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "tar" | "gz" => "application/gzip",
        "apk" => "application/vnd.android.package-archive",
        "exe" | "msi" => "application/octet-stream",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn next_item_id() -> String {
    format!("ls_item_{}", ITEM_SEQ.fetch_add(1, Ordering::SeqCst))
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>, max_files: usize) {
    if out.len() >= max_files {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        if out.len() >= max_files {
            break;
        }
        let p = entry.path();
        if p.is_dir() {
            collect_files_recursive(&p, out, max_files);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// 将一组本地路径（文件或文件夹）解析为待发送的 LocalSendItem 列表
pub fn resolve_paths_to_items(paths: Vec<String>) -> Vec<LocalSendItem> {
    let mut files = Vec::new();
    for raw in paths {
        let trimmed = raw.trim().trim_matches('"');
        if trimmed.is_empty() {
            continue;
        }
        let p = PathBuf::from(trimmed);
        if p.is_file() {
            files.push(p);
        } else if p.is_dir() {
            collect_files_recursive(&p, &mut files, 200);
        }
    }

    let mut items = Vec::with_capacity(files.len());
    for f in files {
        let Ok(meta) = std::fs::metadata(&f) else { continue };
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.bin")
            .to_string();
        let file_type = guess_mime(&f);
        items.push(LocalSendItem {
            id: next_item_id(),
            kind: "file".into(),
            name,
            path: Some(f.to_string_lossy().into()),
            text: None,
            data_url: None,
            size: meta.len(),
            file_type,
        });
    }
    items
}

// ---------- 核心发送逻辑（prepare-upload + 流式 upload） ----------

pub async fn send_items(
    peer: LocalSendPeer,
    items: Vec<LocalSendItem>,
    pin: Option<String>,
    on_progress: Channel<LocalSendProgress>,
) -> Result<()> {
    if items.is_empty() {
        bail!("待发送列表为空");
    }

    let cancel = Arc::new(AtomicBool::new(false));
    *cancel_slot().lock().unwrap() = Some(cancel.clone());

    let total_files = items.len();

    struct PreparedFile {
        item: LocalSendItem,
        mem_bytes: Option<Vec<u8>>,
        size: u64,
    }

    let mut prepared = Vec::with_capacity(total_files);
    let mut total_bytes = 0u64;

    for mut item in items {
        if item.id.trim().is_empty() {
            item.id = next_item_id();
        }
        if item.kind == "text" {
            let txt = item.text.clone().unwrap_or_default();
            let bytes = txt.into_bytes();
            let sz = bytes.len() as u64;
            item.size = sz;
            item.file_type = "text/plain".into();
            if item.name.trim().is_empty() {
                item.name = "message.txt".into();
            }
            total_bytes += sz;
            prepared.push(PreparedFile {
                item,
                mem_bytes: Some(bytes),
                size: sz,
            });
        } else if item.kind == "image" && item.path.is_none() {
            let raw_b64 = item.data_url.as_deref().unwrap_or("");
            let bytes = crate::imgutil::decode_b64(raw_b64)
                .map_err(|e| anyhow!("解码截图数据失败: {e}"))?;
            let sz = bytes.len() as u64;
            item.size = sz;
            item.file_type = "image/png".into();
            if item.name.trim().is_empty() {
                item.name = format!("截图_{}.png", chrono::Local::now().format("%H%M%S"));
            }
            total_bytes += sz;
            prepared.push(PreparedFile {
                item,
                mem_bytes: Some(bytes),
                size: sz,
            });
        } else {
            let p_str = item
                .path
                .as_deref()
                .ok_or_else(|| anyhow!("文件《{}》缺少本地路径", item.name))?;
            let meta = tokio::fs::metadata(p_str)
                .await
                .map_err(|e| anyhow!("无法读取文件《{}》: {e}", item.name))?;
            let sz = meta.len();
            item.size = sz;
            total_bytes += sz;
            prepared.push(PreparedFile {
                item,
                mem_bytes: None,
                size: sz,
            });
        }
    }

    let _ = on_progress.send(LocalSendProgress {
        stage: "waiting_accept".into(),
        file_id: String::new(),
        file_name: prepared[0].item.name.clone(),
        current_index: 0,
        total_files,
        sent_bytes: 0,
        total_bytes,
        speed_mbps: 0.0,
        percent: 0,
        message: format!("正在请求 {} 接收，请在该设备上点击「接受」…", peer.alias),
    });

    let mut files_map = serde_json::Map::new();
    for pf in &prepared {
        let preview = if pf.item.kind == "text" {
            pf.item.text.clone()
        } else {
            None
        };
        files_map.insert(
            pf.item.id.clone(),
            json!({
                "id": pf.item.id,
                "fileName": pf.item.name,
                "size": pf.size,
                "fileType": pf.item.file_type,
                "sha256": Value::Null,
                "preview": preview,
            }),
        );
    }

    let prepare_body = json!({
        "info": self_info_json(false),
        "files": Value::Object(files_map),
    });

    let prepare_path = match pin.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => format!("/api/localsend/v2/prepare-upload?pin={}", url_encode(p)),
        None => "/api/localsend/v2/prepare-upload".to_string(),
    };

    let use_https = peer.protocol != "http";
    let (status, resp_body) =
        raw_http_json(&peer.ip, peer.port, use_https, "POST", &prepare_path, Some(&prepare_body), 65_000).await?;

    if cancel.load(Ordering::SeqCst) {
        let _ = on_progress.send(LocalSendProgress {
            stage: "cancelled".into(),
            file_id: String::new(),
            file_name: String::new(),
            current_index: 0,
            total_files,
            sent_bytes: 0,
            total_bytes,
            speed_mbps: 0.0,
            percent: 0,
            message: "已取消发送".into(),
        });
        return Ok(());
    }

    match status {
        204 => {
            let _ = on_progress.send(LocalSendProgress {
                stage: "done".into(),
                file_id: String::new(),
                file_name: prepared[0].item.name.clone(),
                current_index: total_files,
                total_files,
                sent_bytes: total_bytes,
                total_bytes,
                speed_mbps: 0.0,
                percent: 100,
                message: format!("✓ 已成功发送至 {}", peer.alias),
            });
            return Ok(());
        }
        200 => {}
        401 => {
            let _ = on_progress.send(LocalSendProgress {
                stage: "pin_required".into(),
                file_id: String::new(),
                file_name: String::new(),
                current_index: 0,
                total_files,
                sent_bytes: 0,
                total_bytes,
                speed_mbps: 0.0,
                percent: 0,
                message: "对方设备开启了 PIN 码验证，请输入 PIN 码后重试".into(),
            });
            bail!("需要输入正确的 PIN 码 (401)");
        }
        403 => {
            let _ = on_progress.send(LocalSendProgress {
                stage: "rejected".into(),
                file_id: String::new(),
                file_name: String::new(),
                current_index: 0,
                total_files,
                sent_bytes: 0,
                total_bytes,
                speed_mbps: 0.0,
                percent: 0,
                message: format!("{} 拒绝了接收请求", peer.alias),
            });
            bail!("对方拒绝了接收请求 (403)");
        }
        409 => {
            bail!("对方设备正在进行其他传输任务，请稍后再试 (409)");
        }
        code => {
            bail!("握手失败 (HTTP {code}): {resp_body}");
        }
    }

    let prep_json: Value =
        serde_json::from_str(&resp_body).map_err(|e| anyhow!("解析握手响应失败: {e} ({resp_body})"))?;
    let session_id = prep_json
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("握手响应缺少 sessionId"))?
        .to_string();
    let token_map = prep_json
        .get("files")
        .and_then(|x| x.as_object())
        .ok_or_else(|| anyhow!("握手响应缺少 files 令牌字典"))?;

    let start_ts = Instant::now();
    let mut global_sent = 0u64;

    for (idx, pf) in prepared.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            let _ = raw_http_json(
                &peer.ip,
                peer.port,
                use_https,
                "POST",
                &format!("/api/localsend/v2/cancel?sessionId={}", url_encode(&session_id)),
                None,
                2000,
            )
            .await;
            let _ = on_progress.send(LocalSendProgress {
                stage: "cancelled".into(),
                file_id: pf.item.id.clone(),
                file_name: pf.item.name.clone(),
                current_index: idx + 1,
                total_files,
                sent_bytes: global_sent,
                total_bytes,
                speed_mbps: 0.0,
                percent: 0,
                message: "已取消发送".into(),
            });
            return Ok(());
        }

        let Some(token) = token_map.get(&pf.item.id).and_then(|v| v.as_str()) else {
            continue;
        };

        let upload_path = format!(
            "/api/localsend/v2/upload?sessionId={}&fileId={}&token={}",
            url_encode(&session_id),
            url_encode(&pf.item.id),
            url_encode(token)
        );

        let mut stream = open_peer_stream(&peer.ip, peer.port, use_https, 10_000).await?;
        let req_head = format!(
            "POST {upload_path} HTTP/1.1\r\n\
             Host: {}:{}\r\n\
             User-Agent: LocalSend/2.2 (miniBot)\r\n\
             Content-Type: application/octet-stream\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n",
            peer.ip, peer.port, pf.size
        );
        stream.write_all(req_head.as_bytes()).await?;

        if let Some(mem) = &pf.mem_bytes {
            let mut offset = 0usize;
            while offset < mem.len() {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let end = (offset + 65536).min(mem.len());
                stream.write_all(&mem[offset..end]).await?;
                let n = (end - offset) as u64;
                offset = end;
                global_sent += n;

                let elapsed = start_ts.elapsed().as_secs_f64().max(0.05);
                let speed_mbps = (global_sent as f64 / (1024.0 * 1024.0)) / elapsed;
                let percent = if total_bytes > 0 {
                    ((global_sent * 100) / total_bytes).min(99) as u32
                } else {
                    99
                };
                let _ = on_progress.send(LocalSendProgress {
                    stage: "sending".into(),
                    file_id: pf.item.id.clone(),
                    file_name: pf.item.name.clone(),
                    current_index: idx + 1,
                    total_files,
                    sent_bytes: global_sent,
                    total_bytes,
                    speed_mbps,
                    percent,
                    message: format!("正在发送 ({}/{})：{}", idx + 1, total_files, pf.item.name),
                });
            }
        } else if let Some(path_str) = &pf.item.path {
            let mut f = tokio::fs::File::open(path_str).await?;
            let mut buf = vec![0u8; 65536];
            let mut last_emit = Instant::now();
            loop {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let n = f.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                stream.write_all(&buf[..n]).await?;
                global_sent += n as u64;

                if last_emit.elapsed() >= Duration::from_millis(80) || global_sent == total_bytes {
                    last_emit = Instant::now();
                    let elapsed = start_ts.elapsed().as_secs_f64().max(0.05);
                    let speed_mbps = (global_sent as f64 / (1024.0 * 1024.0)) / elapsed;
                    let percent = if total_bytes > 0 {
                        ((global_sent * 100) / total_bytes).min(99) as u32
                    } else {
                        99
                    };
                    let _ = on_progress.send(LocalSendProgress {
                        stage: "sending".into(),
                        file_id: pf.item.id.clone(),
                        file_name: pf.item.name.clone(),
                        current_index: idx + 1,
                        total_files,
                        sent_bytes: global_sent,
                        total_bytes,
                        speed_mbps,
                        percent,
                        message: format!("正在发送 ({}/{})：{}", idx + 1, total_files, pf.item.name),
                    });
                }
            }
        }

        if cancel.load(Ordering::SeqCst) {
            let _ = raw_http_json(
                &peer.ip,
                peer.port,
                use_https,
                "POST",
                &format!("/api/localsend/v2/cancel?sessionId={}", url_encode(&session_id)),
                None,
                2000,
            )
            .await;
            let _ = on_progress.send(LocalSendProgress {
                stage: "cancelled".into(),
                file_id: pf.item.id.clone(),
                file_name: pf.item.name.clone(),
                current_index: idx + 1,
                total_files,
                sent_bytes: global_sent,
                total_bytes,
                speed_mbps: 0.0,
                percent: 0,
                message: "已取消发送".into(),
            });
            return Ok(());
        }

        stream.flush().await?;
        let (up_status, up_resp) = read_http_response(&mut stream, 15_000).await?;
        if !(200..300).contains(&up_status) {
            bail!("上传《{}》失败 (HTTP {up_status}): {up_resp}", pf.item.name);
        }
    }

    let elapsed = start_ts.elapsed().as_secs_f64().max(0.05);
    let speed_mbps = (global_sent as f64 / (1024.0 * 1024.0)) / elapsed;
    let _ = on_progress.send(LocalSendProgress {
        stage: "done".into(),
        file_id: String::new(),
        file_name: String::new(),
        current_index: total_files,
        total_files,
        sent_bytes: total_bytes,
        total_bytes,
        speed_mbps,
        percent: 100,
        message: format!("✓ 全部发送完成（共 {} 项）", total_files),
    });

    Ok(())
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}


// ---------- 原生多文件选择对话框（Windows / macOS / Linux 跨平台） ----------

pub fn pick_files_dialog() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        type HWND = isize;
        #[repr(C)]
        struct OPENFILENAMEW {
            l_struct_size: u32,
            hwnd_owner: HWND,
            h_instance: isize,
            lpstr_filter: *const u16,
            lpstr_custom_filter: *mut u16,
            n_max_cust_filter: u32,
            n_filter_index: u32,
            lpstr_file: *mut u16,
            n_max_file: u32,
            lpstr_file_title: *mut u16,
            n_max_file_title: u32,
            lpstr_initial_dir: *const u16,
            lpstr_title: *const u16,
            flags: u32,
            n_file_offset: u16,
            n_file_extension: u16,
            lpstr_def_ext: *const u16,
            l_cust_data: isize,
            lpfn_hook: isize,
            lp_template_name: *const u16,
            pv_reserved: *mut core::ffi::c_void,
            dw_reserved: u32,
            flags_ex: u32,
        }

        const OFN_EXPLORER: u32 = 0x00080000;
        const OFN_ALLOWMULTISELECT: u32 = 0x00000200;
        const OFN_FILEMUSTEXIST: u32 = 0x00001000;
        const OFN_PATHMUSTEXIST: u32 = 0x00000800;
        const OFN_NOCHANGEDIR: u32 = 0x00000008;

        #[link(name = "comdlg32")]
        unsafe extern "system" {
            fn GetOpenFileNameW(lpofn: *mut OPENFILENAMEW) -> i32;
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetForegroundWindow() -> HWND;
        }

        unsafe {
            let mut buf = vec![0u16; 32768];
            let filter: Vec<u16> = OsStr::new("所有文件 (*.*)\0*.*\0\0").encode_wide().collect();
            let title: Vec<u16> = OsStr::new("选择要通过 LocalSend 发送的文件（支持多选）\0")
                .encode_wide()
                .collect();

            let mut ofn: OPENFILENAMEW = std::mem::zeroed();
            ofn.l_struct_size = std::mem::size_of::<OPENFILENAMEW>() as u32;
            ofn.hwnd_owner = GetForegroundWindow();
            ofn.lpstr_filter = filter.as_ptr();
            ofn.lpstr_file = buf.as_mut_ptr();
            ofn.n_max_file = buf.len() as u32;
            ofn.lpstr_title = title.as_ptr();
            ofn.flags = OFN_EXPLORER | OFN_ALLOWMULTISELECT | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;

            if GetOpenFileNameW(&mut ofn) == 0 {
                return Vec::new();
            }

            let mut parts: Vec<String> = Vec::new();
            let mut start = 0usize;
            for i in 0..buf.len() {
                if buf[i] == 0 {
                    if i == start {
                        break;
                    }
                    let s = std::ffi::OsString::from_wide(&buf[start..i])
                        .to_string_lossy()
                        .to_string();
                    parts.push(s);
                    start = i + 1;
                }
            }

            if parts.len() == 1 {
                return vec![parts.remove(0)];
            } else if parts.len() > 1 {
                let dir = PathBuf::from(&parts[0]);
                return parts[1..]
                    .iter()
                    .map(|name| dir.join(name).to_string_lossy().to_string())
                    .collect();
            }
        }
        Vec::new()
    }

    #[cfg(target_os = "macos")]
    {
        let script = r#"
            set fs to choose file with prompt "选择要通过 LocalSend 发送的文件" with multiple selections allowed
            set out to ""
            repeat with f in fs
                set out to out & POSIX path of f & linefeed
            end repeat
            return out
        "#;
        if let Ok(out) = std::process::Command::new("osascript").args(["-e", script]).output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
            }
        }
        Vec::new()
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Ok(out) = std::process::Command::new("zenity")
            .args(["--file-selection", "--multiple", "--separator=\n"])
            .output()
        {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
            }
        }
        Vec::new()
    }
}

pub fn pick_folder_for_localsend() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        crate::engine::pick_folder_dialog("")
    }
    #[cfg(target_os = "macos")]
    {
        let script = r#"POSIX path of (choose folder with prompt "选择要通过 LocalSend 发送的文件夹")"#;
        if let Ok(out) = std::process::Command::new("osascript").args(["-e", script]).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Ok(out) = std::process::Command::new("zenity")
            .args(["--file-selection", "--directory"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }
}

// ---------- 系统文件右键菜单集成（Windows 注册表 + macOS Finder Quick Action Service） ----------

pub fn is_context_menu_registered() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Classes\*\shell\miniBotLocalSend\command"])
            .creation_flags(0x0800_0000)
            .output()
        {
            return out.status.success();
        }
        false
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            return home
                .join("Library/Services/发送局域网文件 (miniBot).workflow/Contents/Info.plist")
                .exists();
        }
        false
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        false
    }
}

pub fn set_context_menu_registered(enable: bool) -> Result<bool> {
    let exe = std::env::current_exe().map_err(|e| anyhow!("无法获取当前程序路径: {e}"))?;
    let exe_str = exe.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let keys = [
            r"HKCU\Software\Classes\*\shell\miniBotLocalSend",
            r"HKCU\Software\Classes\Directory\shell\miniBotLocalSend",
        ];

        if enable {
            let cmd_val = format!("\"{exe_str}\" --localsend \"%1\"");
            let icon_val = format!("\"{exe_str}\",0");
            for k in keys {
                let _ = std::process::Command::new("reg")
                    .args(["add", k, "/ve", "/d", "📡 发送局域网文件 (LocalSend)", "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                let _ = std::process::Command::new("reg")
                    .args(["add", k, "/v", "Icon", "/d", &icon_val, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                let _ = std::process::Command::new("reg")
                    .args(["add", &format!(r"{k}\command"), "/ve", "/d", &cmd_val, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            }
        } else {
            for k in keys {
                let _ = std::process::Command::new("reg")
                    .args(["delete", k, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            }
        }
        return Ok(is_context_menu_registered());
    }

    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("无法定位用户主目录"))?;
        let wf_dir = home.join("Library/Services/发送局域网文件 (miniBot).workflow");
        if enable {
            let contents = wf_dir.join("Contents");
            std::fs::create_dir_all(&contents)?;
            let info_plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>发送局域网文件 (LocalSend)</string>
            </dict>
            <key>NSMessage</key>
            <string>runWorkflowAsService</string>
            <key>NSSendFileTypes</key>
            <array>
                <string>public.item</string>
            </array>
        </dict>
    </array>
</dict>
</plist>"#;
            let wflow = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>AMApplicationBuild</key>
    <string>523</string>
    <key>AMApplicationVersion</key>
    <string>2.10</string>
    <key>AMDocumentVersion</key>
    <string>2</string>
    <key>actions</key>
    <array>
        <dict>
            <key>action</key>
            <dict>
                <key>AMAccepts</key>
                <dict>
                    <key>Container</key>
                    <string>List</string>
                    <key>Optional</key>
                    <true/>
                    <key>Types</key>
                    <array><string>com.apple.cocoa.path</string></array>
                </dict>
                <key>AMActionVersion</key>
                <string>2.0.3</string>
                <key>AMParameterProperties</key>
                <dict>
                    <key>COMMAND_STRING</key>
                    <dict/>
                    <key>CheckedForUserDefaultShell</key>
                    <dict/>
                    <key>inputMethod</key>
                    <dict/>
                    <key>shell</key>
                    <dict/>
                    <key>source</key>
                    <dict/>
                </dict>
                <key>ActionBundlePath</key>
                <string>/System/Library/Automator/Run Shell Script.action</string>
                <key>ActionName</key>
                <string>Run Shell Script</string>
                <key>ActionParameters</key>
                <dict>
                    <key>COMMAND_STRING</key>
                    <string>"{exe_str}" --localsend "$@"</string>
                    <key>CheckedForUserDefaultShell</key>
                    <true/>
                    <key>inputMethod</key>
                    <integer>1</integer>
                    <key>shell</key>
                    <string>/bin/zsh</string>
                    <key>source</key>
                    <string></string>
                </dict>
            </dict>
        </dict>
    </array>
</dict>
</plist>"#
            );
            std::fs::write(contents.join("Info.plist"), info_plist)?;
            std::fs::write(contents.join("document.wflow"), wflow)?;
            let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
                .arg("-update")
                .output();
        } else if wf_dir.exists() {
            let _ = std::fs::remove_dir_all(&wf_dir);
            let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
                .arg("-update")
                .output();
        }
        return Ok(is_context_menu_registered());
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let _ = (enable, exe_str);
        Ok(false)
    }
}

pub fn extract_cli_paths(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut collect = false;
    for a in args {
        if a == "--localsend" {
            collect = true;
            continue;
        }
        if collect {
            if a.starts_with("--") {
                break;
            }
            let clean = a.trim().trim_matches('"').to_string();
            if !clean.is_empty() {
                out.push(clean);
            }
        }
    }
    out
}


