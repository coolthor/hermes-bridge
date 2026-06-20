# Hermes Phone Bridge

**繁體中文** · [English](./README.en.md)

> 把 **Muninn** iOS app 連上你自己的 Hermes agent,透過 [iroh](https://www.iroh.computer/) P2P 直連——加密、直連、**零營運**。兩端都你自己跑,路徑上沒有別人。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Built with iroh](https://img.shields.io/badge/built%20with-iroh-7C3AED.svg)](https://www.iroh.computer/)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)]()

![Muninn — 隨時隨地跟家裡的 Hermes 說話](./assets/hero.png)

---

## 這是什麼

`hermes-bridge` 是一支跑在你 Hermes 旁邊的小型 Rust 常駐程式。它把你本機的 Hermes dashboard,經過端到端加密的 iroh 通道,開放給**一支已配對的手機**——讓你在任何地方、用行動網路,跟家裡的 agent 聊天,不需要雲端帳號,中間也沒有任何第三方伺服器。

![Muninn 如何連回你的 agent:iOS app 透過加密的 iroh P2P 通道連到 bridge,bridge 再轉送到你本機 127.0.0.1 上的 Hermes dashboard](./assets/architecture.svg)

## 特色

- **直連 P2P。** 兩端只要有可路由位址(例如行動網路上的 IPv6),iroh 就會打洞出一條 DIRECT 直連;打不到才退而走公共 relay。
- **零營運。** 沒有帳號、沒有你要顧的伺服器、沒有 SaaS。bridge 跟 app 就是整個系統。
- **配對就是安全邊界。** 掃到的 QR 是一次性「介紹信」不是鑰匙——新裝置要你確認指紋後才會進白名單。詳見 [SECURITY.md](./SECURITY.md)。
- **穩定身分。** 持久金鑰讓 bridge 有固定的 NodeId,已配對的手機重連免再掃。
- **媒體傳輸。** 專用通道把手機的圖/語音送給 agent,也把 agent 的圖、影片、音訊、文件送回手機。
- **單一小 binary。** 沒有 runtime;安裝免裝 Rust——installer 會抓你平台對應的編好 binary。

## 需求

- 本機跑著 **Hermes** —— Hermes Desktop,或無頭的 `hermes dashboard` CLI 都行。
- 手機裝 **Muninn** —— [**TestFlight 公開測試連結**](https://testflight.apple.com/join/8mcRtXsm)(點開直接安裝,免申請)。
- **不需要先裝 Rust** —— installer 會自動下載你平台對應的編好 binary(沒有的話才退而本機編)。

## 安裝 —— 一句話丟給你的 Hermes

把這句貼進你自己的 Hermes,它會幫你跑完所有步驟:

> 把 https://github.com/coolthor/hermes-bridge clone 到 ~/hermes-bridge,跑它的 install.sh,然後顯示配對 QR 讓我連手機。

它會:clone repo → 下載對應平台的 bridge binary(免裝 Rust)→ 裝好 `connect-phone` skill → 彈出 QR。然後打開 **Muninn** → 連線 → **掃描 QR Code**。

### 或自己手動跑

```bash
git clone https://github.com/coolthor/hermes-bridge ~/hermes-bridge
bash ~/hermes-bridge/install.sh
```

## 日常使用

- **再顯示一次 QR:** 跟你的 Hermes 說 **「連接手機」**(或 "connect my phone"),`connect-phone` skill 就會重新顯示。
- **重連:** 配對過一次後,手機的 NodeId 會被記住——重開 app 自動重連,免再掃。
- **核准新裝置:** 第一次的裝置會顯示一組碼,用 `run-bridge.sh approve <code>` 確認(bridge 絕不自動信任掃描者)。

## 運作原理

bridge 維持一個被 supervisor 看顧的常駐程式,提供數個 iroh ALPN:

| ALPN | 用途 |
|------|------|
| `hermes-bridge/0`          | 透明轉送本機 Hermes dashboard 的 WebSocket |
| `hermes-bridge-upload/0`   | 手機 → agent 上傳(例如你傳的照片) |
| `hermes-bridge-download/0` | agent → 手機的媒體與檔案(圖、影片、音訊、文件) |

所有流量都由 iroh 的 ed25519 節點身分驗證+加密。bridge 永遠只轉送到 `127.0.0.1`(你的 dashboard),且只服務白名單上的 NodeId。

## 疑難排解

- **一直卡在 `relay`(打不到 `DIRECT`)** —— 打洞需要兩端都有可路由位址,行動網路上通常指 IPv6;對稱/嚴格 NAT 或純 IPv4 路徑會退回 relay(能通,但較慢較容易掉)。這是網路特性,不是 bug。
- **重啟 dashboard 後 bridge「掛了」** —— supervisor(`scripts/run-bridge.sh`)負責保活;若被一起殺掉,重新拉起:
  ```bash
  nohup bash ~/hermes-bridge/scripts/run-bridge.sh >/dev/null 2>&1 &
  ```
- **手機配不上** —— 確認手機上的碼跟你 `approve` 的一致;掃到 QR 的陌生人會顯示**不同**指紋,不要核准。

## 安全

![配對是「介紹信」不是鑰匙:你確認指紋後只有你的裝置進白名單;洩漏的 QR 屬於不同的 NodeId、指紋你不會確認,而且一次性配對碼早已消耗](./assets/pairing.svg)

QR 內容是 `hb1|<iroh-ticket>|<配對碼>` —— 一次性介紹信,**不是鑰匙**。只有實際掃描的那支手機的 NodeId 會被加進白名單;配對碼單次有效且會過期。**洩漏的 QR 沒用**:攻擊者是不同的 NodeId,碼也已消耗。連線由 iroh 的 ed25519 節點身分**端到端加密驗證**,不經任何第三方伺服器,並帶著你的 Hermes dashboard session token。

> ⚠️ **配對一台裝置 = 給它 agent 存取權,而 agent 能在這台機器上跑 shell。** 只配對你自己的裝置。完整模型見 [SECURITY.md](./SECURITY.md)。

## 授權

[MIT](./LICENSE) © 2026 coolthor。建構於 [number 0](https://n0.computer/) 的 [iroh](https://www.iroh.computer/)。
