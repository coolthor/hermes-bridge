//! Test client for hermes-bridge: simulates what the iOS app's loopback proxy
//! does over iroh. Parses `hb1|<ticket>|<code>`, dials the bridge, pairs with
//! the code, then opens a proxy stream and sends a raw HTTP GET — proving the
//! pairing handshake + transparent TCP proxy carry real HTTP end-to-end.
//!
//! Usage: testclient "hb1|endpoint...|CODE"   [path]

use iroh::{Endpoint, endpoint::presets};
use iroh_tickets::{Ticket as _, endpoint::EndpointTicket};
use n0_error::{Result, StdResultExt};

const ALPN: &[u8] = b"hermes-bridge/0";

#[tokio::main]
async fn main() -> Result<()> {
    let payload = std::env::args().nth(1).expect("usage: testclient <hb1|ticket|code> [path]");
    let path = std::env::args().nth(2).unwrap_or_else(|| "/".to_string());

    let parts: Vec<&str> = payload.split('|').collect();
    assert_eq!(parts.len(), 3, "payload must be hb1|ticket|code");
    assert_eq!(parts[0], "hb1", "bad scheme");
    let (ticket_str, code) = (parts[1], parts[2]);

    let addr = EndpointTicket::decode_string(ticket_str).anyerr()?.endpoint_addr().clone();

    let ep = Endpoint::bind(presets::N0).await?;
    println!("my id   : {}", ep.id());
    println!("dialing the bridge...");
    let conn = ep.connect(addr, ALPN).await?;
    println!("connected to {}", conn.remote_id());

    // ── control handshake on the first stream (pair + get session token) ──
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    send.write_all(format!("PAIR {code}").as_bytes()).await.anyerr()?;
    send.finish().anyerr()?;
    let resp = recv.read_to_end(512).await.anyerr()?;
    let resp = String::from_utf8_lossy(&resp);
    let resp = resp.trim();
    if !resp.starts_with("OK") {
        println!("❌ control rejected: {resp}");
        ep.close().await;
        return Ok(());
    }
    let token = resp.strip_prefix("OK").unwrap_or("").trim();
    if token.is_empty() {
        println!("✅ authorized; session token = (none — dashboard not token-gated)");
    } else {
        println!("✅ authorized; session token = {}… ({} chars)", &token[..token.len().min(8)], token.len());
    }

    // ── proxy stream: send a raw HTTP GET, read the response ──
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:9120\r\nConnection: close\r\n\r\n");
    send.write_all(req.as_bytes()).await.anyerr()?;
    // NOTE: do NOT finish() the send half — that half-closes the stream and
    // uvicorn aborts the response (half-close kills HTTP proxying). The real WS
    // client keeps the connection open, so this mirrors real behavior.
    let body = recv.read_to_end(64 * 1024).await.anyerr()?;
    let text = String::from_utf8_lossy(&body);
    let status = text.lines().next().unwrap_or("(no response)");
    println!("\n=== HTTP response through iroh ({} bytes) ===", body.len());
    println!("status: {status}");
    println!("--- first 300 bytes ---\n{}", &text.chars().take(300).collect::<String>());
    println!("\n✅ proxy carried HTTP end-to-end");

    ep.close().await;
    Ok(())
}
