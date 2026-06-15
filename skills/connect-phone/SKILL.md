---
name: connect-phone
description: Start the Hermes phone bridge and show a pairing QR so the user can connect the HermesApp iOS app to this machine's Hermes over iroh P2P. Use when the user says things like "連接手機", "connect my phone", "pair my phone", "show the pairing QR", "我要用手機 app 連", or asks how to get the phone app connected.
version: 1.0.0
platforms: [macos, linux]
metadata:
  hermes:
    tags: [phone, pairing, iroh, bridge, qr, hermesapp, mobile]
---

# Connect Phone — pair the HermesApp iOS app to this Hermes

When the user wants to connect their phone (the **HermesApp** iOS app) to this
machine's Hermes, run the bridge and show them the pairing QR. The bridge proxies
the local Hermes dashboard over iroh P2P; the app scans the QR to connect directly.

## What to do

1. Start (or re-point) the bridge and open the QR on screen:

   ```bash
   DIR="$(cd "${HERMES_HOME:-$HOME/.hermes}/skills/connect-phone" && pwd -P)"
   "$DIR/../../scripts/run-bridge.sh" --open
   ```

   This builds the bridge if needed, detects the live dashboard port, starts the
   bridge with a persistent NodeId, writes the QR to `~/.hermes-bridge/qr.png`
   and opens it, then forks a supervisor that keeps the bridge pointed at the
   right port across Hermes Desktop restarts.

2. Tell the user (繁中):
   - 打開手機上的 **HermesApp** -> 右上角連線 -> **掃描 QR Code** -> 對準螢幕上的 QR。
   - 配對碼 5 分鐘內有效；連上後手機 NodeId 會被記住，**之後重連免再掃**。
   - 要重新顯示 QR，再叫我一次「連接手機」即可。

## Notes

- The QR encodes `hb1|<iroh-ticket>|<pairing-code>` — a one-time introduction, not
  a key. Only the scanning phone's NodeId is allow-listed; a leaked QR is useless
  (the code is single-use + expires, and an attacker is a different NodeId).
- If the script reports the dashboard isn't running, the user must open Hermes
  Desktop first.
- The bridge keeps running in the background — no need to keep this turn open.
