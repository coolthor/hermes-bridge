# Hermes Phone Bridge

Connect the **HermesApp** iOS app to your own Hermes over [iroh](https://www.iroh.computer/)
P2P. The bridge proxies your local Hermes dashboard through an encrypted direct
connection; your phone scans a pairing QR and you chat with your home agent from
anywhere. Zero operator — you run both ends, nobody else is in the path.

## Prerequisites

- **Hermes** running on this machine — either Hermes Desktop, or the headless `hermes dashboard` CLI.
- **Rust toolchain** ([rustup.rs](https://rustup.rs)) to build the bridge.
- **HermesApp** on your phone (TestFlight / App Store).

## Install — one line to your Hermes

Paste this into your own Hermes (it runs the steps for you):

> Clone https://github.com/coolthor/hermes-bridge into ~/hermes-bridge, run its
> install.sh, then show me the pairing QR so I can connect my phone.

That clones the repo, builds the bridge, installs the `connect-phone` skill, and
shows the QR. Then open HermesApp → 連線 → **掃描 QR Code**.

### Or do it manually

```bash
git clone https://github.com/coolthor/hermes-bridge ~/hermes-bridge
bash ~/hermes-bridge/install.sh
```

## After install

Just tell your Hermes **「連接手機」** (or "connect my phone") any time — the
`connect-phone` skill re-shows the QR. Once paired, your phone's NodeId is
remembered, so reconnecting needs no re-scan.

## Security

The QR encodes `hb1|<iroh-ticket>|<pairing-code>` — a one-time introduction, not a
key. Only the scanning phone's NodeId is allow-listed; the pairing code is
single-use and expires. A leaked QR is useless: an attacker is a different NodeId
and the code is already spent.

The connection is authenticated end-to-end by iroh's ed25519 node identities and
carries your Hermes dashboard session token — it never touches a third-party
server.
