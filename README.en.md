# Hermes Phone Bridge

[繁體中文](./README.md) · **English**

Connect the **HermesApp** iOS app to your own Hermes over [iroh](https://www.iroh.computer/) P2P. The bridge proxies your local Hermes dashboard through an encrypted peer-to-peer channel; your phone scans a pairing QR and you chat with your home agent from anywhere. **Zero operator** — you run both ends, nobody else is in the path.

## Prerequisites

- **Hermes** running on this machine — Hermes Desktop, or the headless `hermes dashboard` CLI.
- **HermesApp** on your phone (TestFlight / App Store).
- **No Rust needed** — the installer downloads the prebuilt binary for your platform (and only builds from source if none is available).

## Install — one line to your Hermes

Paste this into your own Hermes and it runs the steps for you:

> Clone https://github.com/coolthor/hermes-bridge into ~/hermes-bridge, run its install.sh, then show me the pairing QR so I can connect my phone.

It clones the repo, downloads the prebuilt bridge binary (no Rust), installs the `connect-phone` skill, and shows the QR. Then open HermesApp → Connect → **Scan QR Code**.

### Or run it yourself

```bash
git clone https://github.com/coolthor/hermes-bridge ~/hermes-bridge
bash ~/hermes-bridge/install.sh
```

## After install

Any time, tell your Hermes **"connect my phone"** (or 「連接手機」) and the `connect-phone` skill shows the QR again. Once paired, your phone's NodeId is remembered, so reconnecting needs no re-scan.

## Security

The QR encodes `hb1|<iroh-ticket>|<pairing-code>` — a one-time introduction, **not a key**. Only the scanning phone's NodeId is allow-listed; the pairing code is single-use and expires. **A leaked QR is useless**: an attacker is a different NodeId and the code is already spent. The connection is end-to-end authenticated by iroh's ed25519 node identities, never touches a third-party server, and carries your Hermes dashboard session token.
