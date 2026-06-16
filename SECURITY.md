# Security model

The bridge exposes your local Hermes dashboard to a paired phone over iroh P2P.
**Pairing a device grants it agent access — which can run shell on this machine.**
So the pairing gate is the entire security boundary. Only pair devices you own.

## What protects you

- **Cryptographic peer identity.** Every connection is authenticated by iroh's
  ed25519 NodeId (`connection.remote_id()`, unforgeable). Only NodeIds on the
  on-disk allowlist may proxy.
- **Pairing is not first-scanner-wins.** A QR scan alone does NOT grant access. A
  new device gets a one-time code on screen; the bridge replies `PENDING <fp>`
  (a short fingerprint of its NodeId) and queues it — it is **not** allow-listed
  until the operator confirms that fingerprint (the code shown on *their* phone)
  via `run-bridge.sh approve <code>`. A stranger who scanned the QR shows a
  different fingerprint, which the operator won't approve. `approve` refuses if
  two pending devices share a code (collision/attack).
- **Owner-only state.** `~/.hermes-bridge/` is `0700`; the secret key, allowlist,
  pending list, QR, and uploads are `0600`. The pairing code is never written to
  the log. Reads/writes of the allowlist and pending list are serialised with a
  cross-process lock.
- **Resource limits.** Concurrent connections are capped; handshake/upload reads
  time out; wrong-code guesses burn the pairing window; the pending queue rejects
  (rather than evicts) once full, so a flooder can't push out your real device.

## Known, accepted limitations

- **Single-user-machine assumption.** The Hermes **dashboard session token** lives
  in the dashboard process's environment and is served to localhost — this is
  Hermes' own design, not introduced here. Any process running **as the same
  user** can already obtain it. The bridge reads it from its inherited environment
  (or, as a fallback, the dashboard process env). Treat the host as a trusted,
  single-user machine; don't run untrusted local software next to your agent.
- **Pairing-window DoS is recoverable, not impossible.** A party who already has
  the QR can degrade the pairing window (fill the pending queue / open
  connections). It cannot get authorized without your fingerprint confirmation.
  Recover by re-running the connect step (a fresh window clears pending).

## Reporting

Open a GitHub issue (omit secrets/tokens) or contact the maintainer privately for
anything that could bypass the pairing gate.
