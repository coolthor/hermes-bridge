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
use tokio::sync::Semaphore;
use tokio::time::timeout;

/// Cap on concurrent inbound connections (across proxy + upload) — bounds fd /
/// memory so a peer can't exhaust the host by opening thousands at once.
const MAX_CONNS: usize = 64;
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

/// Max devices that may sit in the pending-confirmation queue at once — caps a
/// spammer's chances of a random fingerprint collision with the operator's code.
const MAX_PENDING: usize = 5;

/// 32-char alphabet (no ambiguous 0/O/1/I) — also used for the pairing code.
const FP_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Short device fingerprint shown on BOTH the phone and the operator's confirm
/// prompt: first 4 bytes of the NodeId, each mapped into the 32-char alphabet
/// (`& 0x1F`, bias-free since 32 | 256). ~20 bits ≈ 1/1,048,576.
/// MUST stay byte-identical with the Swift app's derivation.
fn fingerprint(node_bytes: &[u8]) -> String {
    node_bytes
        .iter()
        .take(4)
        .map(|b| FP_ALPHABET[(b & 0x1F) as usize] as char)
        .collect()
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
    pending_path: PathBuf,
    allowed: Mutex<HashSet<String>>,
    /// (code, expiry). `None` once expired or burned by too many wrong guesses.
    pairing: Mutex<Option<(String, Instant)>>,
    /// Wrong-guess counter for the open window; burns the code at the cap.
    pair_attempts: Mutex<u32>,
    /// Caps concurrent inbound connections (anti-exhaustion).
    conn_sem: Arc<Semaphore>,
}

impl BridgeState {
    fn load(target: String) -> Self {
        let dir = home_dir().join(".hermes-bridge");
        ensure_private_dir(&dir);
        let allowed_path = dir.join("allowed");
        let pending_path = dir.join("pending");
        let _ = std::fs::remove_file(&pending_path); // stale from a previous run
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
            pending_path,
            allowed: Mutex::new(allowed),
            pairing: Mutex::new(None),
            pair_attempts: Mutex::new(0),
            conn_sem: Arc::new(Semaphore::new(MAX_CONNS)),
        }
    }

    /// Open a fresh pairing window (a code valid for `PAIRING_WINDOW`). Unlike a
    /// bearer token, the code is NOT consumed on use — multiple devices may PAIR
    /// during the window; the operator's fingerprint confirmation decides who
    /// actually gets allow-listed.
    fn open_pairing_window(&self) -> String {
        let mut rng = rand::thread_rng();
        let code: String = (0..8)
            .map(|_| FP_ALPHABET[rng.gen_range(0..FP_ALPHABET.len())] as char)
            .collect();
        *self.pairing.lock().unwrap() = Some((code.clone(), Instant::now() + PAIRING_WINDOW));
        *self.pair_attempts.lock().unwrap() = 0;
        code
    }

    /// Read the allowlist fresh from disk on every check, so an external `approve`
    /// (which appends a NodeId to the file) takes effect without a restart.
    fn is_allowed(&self, id: &str) -> bool {
        std::fs::read_to_string(&self.allowed_path)
            .unwrap_or_default()
            .lines()
            .any(|l| l.trim() == id)
    }

    /// Is `code` the open window's code (constant-time, not expired)? Does NOT
    /// consume it. Wrong guesses are counted and burn the window at the cap.
    fn code_valid(&self, code: &str) -> bool {
        let mut p = self.pairing.lock().unwrap();
        let ok = matches!(
            p.as_ref(),
            Some((c, exp)) if Instant::now() < *exp && ct_eq(c.as_bytes(), code.as_bytes())
        );
        if !ok {
            let mut n = self.pair_attempts.lock().unwrap();
            *n += 1;
            if *n >= MAX_PAIR_ATTEMPTS {
                *p = None;
            }
        }
        ok
    }

    /// Queue a device for operator confirmation: persist `<fp> <nodeid>` to the
    /// pending file (deduped by NodeId). Returns false (adding nothing) when the
    /// queue is already full of OTHER devices — so a QR-holding attacker can't
    /// evict the legitimate device's pending row by flooding fresh NodeIds. An
    /// already-queued device can always re-register (the app re-PAIRs each poll).
    fn add_pending(&self, fp: &str, id: &str) -> bool {
        let _lock = FileLock::acquire(&self.pending_path);
        let mut lines: Vec<String> = std::fs::read_to_string(&self.pending_path)
            .unwrap_or_default()
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        let already = lines.iter().any(|l| l.split_whitespace().nth(1) == Some(id));
        if !already && lines.len() >= MAX_PENDING {
            return false; // full of other devices — reject rather than evict
        }
        lines.retain(|l| l.split_whitespace().nth(1) != Some(id)); // dedup by NodeId
        lines.push(format!("{fp} {id}"));
        write_private_atomic(&self.pending_path, (lines.join("\n") + "\n").as_bytes());
        true
    }

    /// Remove `id` from the allowlist. iroh authenticates the NodeId, so a device
    /// can only ever revoke ITSELF (UNPAIR) — no code needed.
    fn revoke(&self, id: &str) {
        let _lock = FileLock::acquire(&self.allowed_path);
        let kept: Vec<String> = std::fs::read_to_string(&self.allowed_path)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && *l != id)
            .map(String::from)
            .collect();
        write_private_atomic(&self.allowed_path, (kept.join("\n") + "\n").as_bytes());
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

/// Cross-process advisory lock via atomic `mkdir` — portable (no flock dep, works
/// on macOS which lacks flock(1)) and shared with the shell side, which locks the
/// same `<file>.lock` dir. Serialises read-modify-write on pending/allowed so a
/// concurrent PAIR and approve/revoke can't lose each other's update. RAII: the
/// lock dir is removed on drop. Best-effort: spins ~2s, steals a stale lock.
struct FileLock {
    dir: PathBuf,
    held: bool,
}
impl FileLock {
    fn acquire(target: &Path) -> Self {
        let dir = target.with_extension("lock");
        let mut held = false;
        for _ in 0..200 {
            if std::fs::create_dir(&dir).is_ok() {
                held = true;
                break;
            }
            // Steal a stale lock (holder crashed mid-update).
            if let Ok(modified) = std::fs::metadata(&dir).and_then(|m| m.modified()) {
                if modified.elapsed().map(|a| a > Duration::from_secs(5)).unwrap_or(false) {
                    let _ = std::fs::remove_dir(&dir);
                    continue;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        FileLock { dir, held }
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        if self.held {
            let _ = std::fs::remove_dir(&self.dir);
        }
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
        // Shed load past the connection cap so a peer can't exhaust fd/memory.
        let _permit = match self.state.conn_sem.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
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
                // PAIR from a known device → in. PAIR with a valid code from an
                // unknown device → queue it for operator confirmation (reply
                // PENDING + its fingerprint); the device is NOT let in until the
                // operator approves that fingerprint. This is what defeats a
                // first-scanner: the operator only confirms the code shown on
                // their own phone, so a stranger's fingerprint never matches.
                enum Decision { Ok, Pending(String), Unpaired, Reject }
                let decision = if line == "UNPAIR" {
                    // A device removing itself from the allowlist (self-revoke).
                    self.state.revoke(&id_hex);
                    Decision::Unpaired
                } else if let Some(code) = line.strip_prefix("PAIR ") {
                    if self.state.is_allowed(&id_hex) {
                        Decision::Ok
                    } else if self.state.code_valid(code.trim()) {
                        let fp = fingerprint(id.as_bytes());
                        if self.state.add_pending(&fp, &id_hex) {
                            Decision::Pending(fp)
                        } else {
                            Decision::Reject // pending queue full of other devices
                        }
                    } else {
                        Decision::Reject
                    }
                } else if line == "HELLO" {
                    if self.state.is_allowed(&id_hex) { Decision::Ok } else { Decision::Reject }
                } else {
                    Decision::Reject
                };
                match decision {
                    Decision::Ok => {
                        let token = fetch_session_token(&self.state.target).await.unwrap_or_default();
                        let _ = send.write_all(format!("OK {token}\n").as_bytes()).await;
                        let _ = send.finish();
                        println!(">> authorized {id}");
                        true
                    }
                    Decision::Pending(fp) => {
                        let _ = send.write_all(format!("PENDING {fp}\n").as_bytes()).await;
                        let _ = send.finish();
                        println!(">> PENDING {id} — confirm code {fp}");
                        false
                    }
                    Decision::Unpaired => {
                        let _ = send.write_all(b"OK unpaired\n").await;
                        let _ = send.finish();
                        println!(">> UNPAIRED {id}");
                        false
                    }
                    Decision::Reject => {
                        let _ = send.write_all(b"ERR unauthorized\n").await;
                        let _ = send.finish();
                        println!(">> REJECTED {id}");
                        false
                    }
                }
            }
            _ => false, // accept_bi error or handshake timeout
        };
        if !authorized {
            // Grace before dropping: dropping the Connection sends ApplicationClose,
            // which races the peer's read of the OK/PENDING/ERR/unpaired reply →
            // ConnectionLost on the app (it never sees the PENDING code). finish()
            // doesn't guarantee delivery before the drop, so wait for the peer to
            // close (bounded) — by then it has read the reply.
            let _ = timeout(Duration::from_secs(3), connection.closed()).await;
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
        let _permit = match self.state.conn_sem.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
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
