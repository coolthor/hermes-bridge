---
name: connect-phone
description: Pair the Muninn iOS app to this machine's Hermes over iroh P2P — show the pairing QR, then confirm the device by the code shown on the phone. Also the path for moving your daily interface from Telegram/Discord to Muninn. Use when the user says "連接手機", "connect my phone", "pair my phone", "show the pairing QR", "我要用手機 app 連", "我想改用 Muninn", "從 Telegram 換到 app", or asks how to get the phone app connected.
version: 2.1.0
platforms: [macos, linux]
metadata:
  hermes:
    tags: [phone, pairing, iroh, bridge, qr, muninn, mobile, telegram-migration]
---

# Connect Phone — pair the Muninn iOS app to this Hermes

When the user wants to connect their phone (the **Muninn** iOS app), show the
pairing QR, then confirm the device by the code shown on the phone. The bridge
proxies the local Hermes dashboard over iroh P2P.

This is also how someone **moves their daily interface from Telegram/Discord to
Muninn** — they ask over the chat they're already in, you send the QR back into
that same chat, they scan it with Muninn.

Resolve the bridge script once:

```bash
DIR="$(cd "${HERMES_HOME:-$HOME/.hermes}/skills/connect-phone" && pwd -P)"
RB="$DIR/../../scripts/run-bridge.sh"
```

## Step 1 — show the QR

**Pick the delivery based on where the user is:**

### Case A — user is on a messaging channel (Telegram, Discord, …) — DEFAULT

This is the common case for moving *from* Telegram *to* Muninn: the user can't see
this computer's screen, so the QR must appear **inline in the chat**.

```bash
"$RB"
```

(No `--open` — there is no desktop here to open a window on.) The bridge writes the
QR to `~/.hermes-bridge/qr.png`. To deliver it, **write that path as a bare path on
its own line in your reply** — Hermes's gateway auto-attaches bare local image paths
to the channel, so it shows up as an inline photo the user can scan:

```
~/.hermes-bridge/qr.png
```

⚠️ **Write the path bare — do NOT wrap it in backticks or a code block.** The
gateway's media extractor deliberately skips backticked paths (a secrets guard), so
a backticked path will NOT be attached. The bare path is intentional here.

Then tell the user (繁中):
- 打開手機 **Muninn** → 連線 → **掃描 QR Code** → 對準上面這張 QR。

### Case B — user is at this machine's screen (local desktop / TUI)

```bash
"$RB" --open
```

This opens the QR PNG in the desktop image viewer. If there's no GUI (pure TUI over
SSH), the script still prints the path — point the user at `~/.hermes-bridge/qr.png`
to open and scan manually.

Then tell the user (繁中):
- 打開手機 **Muninn** → 連線 → **掃描 QR Code** → 對準螢幕上的 QR。

## Step 2 — confirm the pairing (required for a NEW phone)

After the user scans, their phone shows a **4-character code** and waits. Ask the
user to read that code off their phone, then run:

```bash
"$RB" approve <CODE>
```

Replace `<CODE>` with exactly what the user reports from their phone. The phone
connects on its next auto-retry (a few seconds).

**This is the security check.** Only approve the code the user reads off THEIR OWN
phone — that is what stops someone else who scanned the QR from connecting. If
`approve` says multiple devices share the code, do NOT approve; re-show the QR.

This second factor is also why sending the QR over Telegram (Case A) is safe: a QR
that leaks in transit is useless on its own — pairing only completes when you
approve the code shown on the real phone.

An **already-paired** phone reconnects automatically with no code — Step 2 is only
for a new device.

## Managing paired devices

- `"$RB" list` — show paired devices (NodeId prefixes).
- `"$RB" revoke <nodeid-prefix>` — remove one device (use a prefix from `list`).
- `"$RB" revoke all` — unpair everything.

## Notes

- The QR encodes `hb1|<iroh-ticket>|<pairing-code>` — a one-time introduction, not
  a key. The fingerprint confirmation (Step 2) binds pairing to the physical phone.
- If the script says the dashboard isn't running, the user must open Hermes
  Desktop first, or run `hermes dashboard`.
- The bridge keeps running in the background — no need to keep this turn open.
- Don't have Muninn yet? TestFlight public link: https://testflight.apple.com/join/8mcRtXsm
