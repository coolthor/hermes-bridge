---
name: connect-phone
description: Pair the HermesApp iOS app to this machine's Hermes over iroh P2P — show the pairing QR, then confirm the device by the code shown on the phone. Use when the user says "連接手機", "connect my phone", "pair my phone", "show the pairing QR", "我要用手機 app 連", or asks how to get the phone app connected.
version: 2.0.0
platforms: [macos, linux]
metadata:
  hermes:
    tags: [phone, pairing, iroh, bridge, qr, hermesapp, mobile]
---

# Connect Phone — pair the HermesApp iOS app to this Hermes

When the user wants to connect their phone (the **HermesApp** iOS app), show the
pairing QR, then confirm the device by the code shown on the phone. The bridge
proxies the local Hermes dashboard over iroh P2P.

Resolve the bridge script once:

```bash
DIR="$(cd "${HERMES_HOME:-$HOME/.hermes}/skills/connect-phone" && pwd -P)"
RB="$DIR/../../scripts/run-bridge.sh"
```

## Step 1 — show the QR

```bash
"$RB" --open
```

This starts the bridge and opens the pairing QR. Tell the user (繁中):
- 打開手機 **HermesApp** → 連線 → **掃描 QR Code** → 對準螢幕上的 QR。

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
