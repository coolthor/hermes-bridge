# Hermes Phone Bridge

**繁體中文** · [English](./README.en.md)

把 **HermesApp** iOS app 連上你自己的 Hermes,透過 [iroh](https://www.iroh.computer/) P2P 直連。bridge 把你本機的 Hermes dashboard 經加密點對點通道轉給手機;手機掃配對 QR,就能在任何地方跟家裡的 agent 聊天。**零營運**——兩端都你自己跑,路徑上沒有別人。

## 需求

- 本機跑著 **Hermes** —— Hermes Desktop,或無頭的 `hermes dashboard` CLI 都行。
- 手機裝 **HermesApp**(TestFlight / App Store)。
- **不需要先裝 Rust** —— installer 會自動下載你平台對應的編好 binary(沒有的話才退而本機編)。

## 安裝 —— 一句話丟給你的 Hermes

把這句貼進你自己的 Hermes,它會幫你跑完所有步驟:

> 把 https://github.com/coolthor/hermes-bridge clone 到 ~/hermes-bridge,跑它的 install.sh,然後顯示配對 QR 讓我連手機。

它會:clone repo → 下載對應平台的 bridge binary(免裝 Rust)→ 裝好 `connect-phone` skill → 彈出 QR。然後打開 HermesApp → 連線 → **掃描 QR Code**。

### 或自己手動跑

```bash
git clone https://github.com/coolthor/hermes-bridge ~/hermes-bridge
bash ~/hermes-bridge/install.sh
```

## 安裝後

之後任何時候跟你的 Hermes 說 **「連接手機」**,`connect-phone` skill 就會重新顯示 QR。配對過一次後,手機的 NodeId 會被記住,**重連免再掃**。

## 安全

QR 內容是 `hb1|<iroh-ticket>|<配對碼>` —— 一次性介紹信,**不是鑰匙**。只有實際掃描的那支手機的 NodeId 會被加進白名單;配對碼單次有效且會過期。**洩漏的 QR 沒用**:攻擊者是不同的 NodeId,碼也已消耗。連線由 iroh 的 ed25519 節點身分**端到端加密驗證**,不經任何第三方伺服器,並帶著你的 Hermes dashboard session token。
