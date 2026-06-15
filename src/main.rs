//! hermes-bridge: expose a local TCP service (the Hermes dashboard on
//! 127.0.0.1:9119, HTTP + WebSocket) over iroh, with NodeId-allowlist pairing.
//!
//! D1: each incoming iroh bi-stream is proxied 1:1 to a fresh TCP connection to
//!     the target, byte-for-byte (HTTP + WebSocket pass through transparently).
//! D2: security. iroh authenticates every peer by its ed25519 NodeId (the
//!     connection's remote_id is cryptographically verified, unforgeable).
//!     - A *paired* NodeId (in the on-disk allowlist) connects and proxies.
//!     - An *unpaired* NodeId must first present the one-time pairing code
//!       (shown in the QR, valid for a short window, single-use). On success
//!       its NodeId is added to the allowlist; afterwards the QR/code is dead,
//!       so a leaked QR is useless — the real key is the device's NodeId.
//!
//! TODO(D3): hand the dashboard session token to a freshly paired device.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{Result, StdResultExt};
use qrcode::{QrCode, render::unicode};
use rand::Rng;
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

/// Max wrong pairing-code guesses before the open window is burned.
const MAX_PAIR_ATTEMPTS: u32 = 8;
/// Bound the control handshake / upload reads so a peer can't hold a stream open.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Create `dir` (if needed) and lock it to owner-only (0700 on unix).
fn ensure_private_dir(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
}

/// Restrict a file to owner read/write only (0600 on unix).
fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Constant-time byte comparison so the pairing-code check leaks no timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

const ALPN: &[u8] = b"hermes-bridge/0";
/// Separate ALPN for image uploads — keeps the transparent proxy untouched.
const UPLOAD_ALPN: &[u8] = b"hermes-bridge-upload/0";
const DEFAULT_TARGET: &str = "127.0.0.1:9119";
/// QR payload scheme: `hb1|<ticket>|<pairing_code>`.
const QR_SCHEME: &str = "hb1";
/// How long a fresh pairing code stays valid.
const PAIRING_WINDOW: Duration = Duration::from_secs(300);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hermes_bridge=info,iroh=warn".into()),
        )
        .init();

    let target = std::env::var("HERMES_DASHBOARD").unwrap_or_else(|_| DEFAULT_TARGET.to_string());
    let state = Arc::new(BridgeState::load(target.clone()));

    // Open a single-use pairing window so a phone can pair this run.
    let code = state.open_pairing_window();

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(load_or_create_secret())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Default)
        .bind()
        .await?;
    endpoint.online().await;

    let ticket = EndpointTicket::new(endpoint.addr()).to_string();
    let qr_payload = format!("{QR_SCHEME}|{ticket}|{code}");

    let qr = QrCode::new(qr_payload.as_bytes()).anyerr()?;
    let term = qr
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    // QR artifacts live in the owner-only ~/.hermes-bridge dir (NOT world-readable
    // /tmp), so a local user can't lift the pairing code/ticket off disk.
    let bridge_dir = home_dir().join(".hermes-bridge");
    let png_path = bridge_dir.join("qr.png");
    write_qr_png(&qr, &png_path).anyerr()?;
    restrict_file(&png_path);
    // Also persist the terminal (unicode) QR so a TUI/terminal user can `cat` and
    // scan it in place instead of opening the PNG in a GUI viewer.
    let txt_path = bridge_dir.join("qr.txt");
    let _ = std::fs::write(&txt_path, &term);
    restrict_file(&txt_path);

    let paired_count = state.allowed.lock().unwrap().len();
    println!("\n================ HERMES BRIDGE READY ================");
    println!("EndpointId    : {}", endpoint.id());
    println!("Exposing      : {target}  (over iroh)");
    println!("Paired devices: {paired_count}");
    println!("QR PNG        : {}", png_path.display());
    // The pairing code + payload + scannable terminal QR are SECRETS — print them
    // only to an interactive terminal, never when stdout is redirected to a log
    // file (where another local user could read them during the pairing window).
    if std::io::stdout().is_terminal() {
        println!("Pairing code  : {code}   (valid {}s, single use)", PAIRING_WINDOW.as_secs());
        println!("QR payload    : {qr_payload}");
        println!("\n{term}");
    } else {
        println!("Pairing window: open {}s (scan the QR PNG above)", PAIRING_WINDOW.as_secs());
    }
    println!("=====================================================\n");

    let router = Router::builder(endpoint)
        .accept(ALPN, Proxy { state: state.clone() })
        .accept(UPLOAD_ALPN, Upload { state })
        .spawn();

    println!("bridging... (Ctrl-C to quit)\n");
    tokio::signal::ctrl_c().await.anyerr()?;

    router.shutdown().await.anyerr()?;
    Ok(())
}

// ─────────────────────────── pairing state ───────────────────────────

struct BridgeState {
    target: String,
    allowed_path: PathBuf,
    allowed: Mutex<HashSet<String>>,
    /// (code, expiry). `None` once consumed or expired.
    pairing: Mutex<Option<(String, Instant)>>,
    /// Wrong-guess counter for the open window; burns the code at the cap.
    pair_attempts: Mutex<u32>,
}

impl BridgeState {
    fn load(target: String) -> Self {
        let dir = home_dir().join(".hermes-bridge");
        ensure_private_dir(&dir);
        let allowed_path = dir.join("allowed");
        let allowed: HashSet<String> = std::fs::read_to_string(&allowed_path)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        Self {
            target,
            allowed_path,
            allowed: Mutex::new(allowed),
            pairing: Mutex::new(None),
            pair_attempts: Mutex::new(0),
        }
    }

    /// Generate a fresh single-use pairing code valid for `PAIRING_WINDOW`.
    fn open_pairing_window(&self) -> String {
        const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no ambiguous chars
        let mut rng = rand::thread_rng();
        let code: String = (0..8)
            .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
            .collect();
        *self.pairing.lock().unwrap() = Some((code.clone(), Instant::now() + PAIRING_WINDOW));
        code
    }

    fn is_allowed(&self, id: &str) -> bool {
        self.allowed.lock().unwrap().contains(id)
    }

    /// Validate `code` against the open pairing window; on success register
    /// `id` (single-use: the window is consumed) and persist the allowlist.
    /// Wrong guesses are counted and the window is burned at `MAX_PAIR_ATTEMPTS`,
    /// so an attacker who knows the NodeId can't brute-force the code.
    fn try_pair(&self, id: &str, code: &str) -> bool {
        {
            let mut p = self.pairing.lock().unwrap();
            let valid = matches!(
                p.as_ref(),
                Some((c, exp)) if Instant::now() < *exp && ct_eq(c.as_bytes(), code.as_bytes())
            );
            if valid {
                *p = None; // consume — single use
            } else {
                // Count the miss; burn the window once guesses hit the cap.
                let mut n = self.pair_attempts.lock().unwrap();
                *n += 1;
                if *n >= MAX_PAIR_ATTEMPTS {
                    *p = None;
                }
                return false;
            }
        }
        let mut a = self.allowed.lock().unwrap();
        a.insert(id.to_string());
        let dump = a.iter().cloned().collect::<Vec<_>>().join("\n");
        write_private_atomic(&self.allowed_path, (dump + "\n").as_bytes());
        true
    }
}

/// Persist `data` to `path` atomically (temp file + rename) with 0600 perms, so
/// a crash mid-write can't corrupt the allowlist and other users can't read it.
fn write_private_atomic(path: &Path, data: &[u8]) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, data).is_ok() {
        restrict_file(&tmp);
        let _ = std::fs::rename(&tmp, path);
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Stable bridge identity: persist the secret key so the NodeId/ticket survives
/// restarts → a phone that paired once never has to re-scan.
fn load_or_create_secret() -> SecretKey {
    let path = home_dir().join(".hermes-bridge").join("secret");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return SecretKey::from_bytes(&arr);
        }
    }
    let sk = SecretKey::generate();
    if let Some(dir) = path.parent() {
        ensure_private_dir(dir);
    }
    // The long-term ed25519 identity must be owner-only (0600) — another local
    // user reading it could clone/spoof this bridge's NodeId.
    write_private_atomic(&path, &sk.to_bytes());
    sk
}

// ─────────────────────────── proxy handler ───────────────────────────

#[derive(Clone)]
struct Proxy {
    state: Arc<BridgeState>,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Proxy")
    }
}

impl ProtocolHandler for Proxy {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id();
        let id_hex = id.to_string();

        // First stream = control handshake. The app sends either:
        //   `PAIR <code>`  — new device presenting the one-time pairing code
        //   `HELLO`        — already-paired device reconnecting
        // On success we reply `OK <session_token>` so the app can authenticate
        // to /api/ws (which expects `?token=<session_token>`).
        let authorized = match timeout(HANDSHAKE_TIMEOUT, connection.accept_bi()).await {
            Ok(Ok((mut send, mut recv))) => {
                // Bound the read so a peer can't hold the control stream open.
                let buf = match timeout(HANDSHAKE_TIMEOUT, recv.read_to_end(256)).await {
                    Ok(Ok(b)) => b,
                    _ => Vec::new(),
                };
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim();
                let ok = if let Some(code) = line.strip_prefix("PAIR ") {
                    self.state.is_allowed(&id_hex) || self.state.try_pair(&id_hex, code.trim())
                } else if line == "HELLO" {
                    self.state.is_allowed(&id_hex)
                } else {
                    false
                };
                if ok {
                    let token = fetch_session_token(&self.state.target).await.unwrap_or_default();
                    let _ = send.write_all(format!("OK {token}\n").as_bytes()).await;
                    let _ = send.finish();
                    println!(">> authorized {id}");
                    true
                } else {
                    let _ = send.write_all(b"ERR unauthorized\n").await;
                    let _ = send.finish();
                    println!(">> REJECTED {id}");
                    false
                }
            }
            _ => false, // accept_bi error or handshake timeout
        };
        if !authorized {
            return Ok(());
        }

        // Proxy every subsequent bi-stream to the target.
        loop {
            match connection.accept_bi().await {
                Ok((send, recv)) => {
                    let target = self.state.target.clone();
                    tokio::spawn(proxy_one(send, recv, target));
                }
                Err(_) => break,
            }
        }
        println!(">> connection closed: {id}");
        Ok(())
    }
}

/// Fetch the dashboard's session token by scraping the injected
/// `window.__HERMES_SESSION_TOKEN__="..."` from its served index.html.
/// Returns None if the dashboard is unreachable or not token-gated.
async fn fetch_session_token(target: &str) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Prefer an explicitly-provided token (the Hermes Desktop backend serves no
    // scrapeable index.html — its SPA lives in Electron, so GET / 404s).
    if let Ok(t) = std::env::var("HERMES_SESSION_TOKEN") {
        if !t.is_empty() {
            return Some(t);
        }
    }
    let mut tcp = tokio::net::TcpStream::connect(target).await.ok()?;
    tcp.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut buf = Vec::new();
    tcp.read_to_end(&mut buf).await.ok()?;
    let text = String::from_utf8_lossy(&buf);
    let marker = "window.__HERMES_SESSION_TOKEN__=\"";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Image upload protocol (separate ALPN). Only paired NodeIds may upload.
/// Each stream carries `UPLOAD <name>\n<bytes>`; we save it under
/// $HERMES_HOME/images/ and reply `OK <server_path>` for `image.attach`.
#[derive(Clone)]
struct Upload {
    state: Arc<BridgeState>,
}

impl std::fmt::Debug for Upload {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Upload")
    }
}

impl ProtocolHandler for Upload {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id().to_string();
        if !self.state.is_allowed(&id) {
            return Ok(()); // only paired devices may upload
        }
        loop {
            match connection.accept_bi().await {
                Ok((mut send, mut recv)) => {
                    let data = match timeout(UPLOAD_TIMEOUT, recv.read_to_end(25 * 1024 * 1024)).await {
                        Ok(Ok(b)) => b,
                        _ => Vec::new(),
                    };
                    let reply = match parse_and_save_upload(&data) {
                        Some(path) => {
                            println!(">> uploaded image -> {path}");
                            format!("OK {path}\n")
                        }
                        None => "ERR upload failed\n".to_string(),
                    };
                    let _ = send.write_all(reply.as_bytes()).await;
                    let _ = send.finish();
                }
                Err(_) => break,
            }
        }
        Ok(())
    }
}

fn parse_and_save_upload(data: &[u8]) -> Option<String> {
    let nl = data.iter().position(|&b| b == b'\n')?;
    let header = std::str::from_utf8(&data[..nl]).ok()?;
    let name = header.strip_prefix("UPLOAD ")?.trim();
    let body = &data[nl + 1..];
    if body.is_empty() {
        return None;
    }
    let home = std::env::var("HERMES_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".hermes"));
    let dir = home.join("images");
    std::fs::create_dir_all(&dir).ok()?;
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let fname = if safe.is_empty() { "upload.png".to_string() } else { safe };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("app_{stamp}_{fname}"));
    std::fs::write(&path, body).ok()?;
    restrict_file(&path);
    Some(path.to_string_lossy().to_string())
}

/// Pump one iroh bi-stream <-> one TCP connection to `target`, both directions.
async fn proxy_one(mut send: SendStream, mut recv: RecvStream, target: String) {
    let tcp = match tokio::net::TcpStream::connect(&target).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("bridge: TCP connect {target} failed: {e}");
            let _ = send.finish();
            return;
        }
    };
    let (mut tcp_r, mut tcp_w) = tcp.into_split();

    let up = async {
        let _ = tokio::io::copy(&mut recv, &mut tcp_w).await;
        let _ = tcp_w.shutdown().await;
    };
    let down = async {
        let _ = tokio::io::copy(&mut tcp_r, &mut send).await;
        let _ = send.finish();
    };
    tokio::join!(up, down);
}

/// Render the QR code to a scaled, quiet-zoned grayscale PNG (white background).
fn write_qr_png(code: &QrCode, path: &Path) -> Result<(), image::ImageError> {
    let w = code.width();
    let colors = code.to_colors();
    let scale = 10usize;
    let quiet = 4usize;
    let dim = ((w + 2 * quiet) * scale) as u32;
    let mut img = image::GrayImage::from_pixel(dim, dim, image::Luma([255u8]));
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = ((quiet + x) * scale + dx) as u32;
                        let py = ((quiet + y) * scale + dy) as u32;
                        img.put_pixel(px, py, image::Luma([0u8]));
                    }
                }
            }
        }
    }
    img.save(path)
}
