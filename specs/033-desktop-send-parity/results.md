# 033 · Desktop Send Parity — results

分支 `033-desktop-send-parity`,叠在 `032-desktop-money-wiring` 上(032 的 44–47
四刀先修掉了刷新、隐藏余额、网络筛选和网络设置四组)。

**闸门**(与 032 相同,原样跑):

```bash
cd app-desktop/vela-wallet
cargo fmt --all --check && cargo test --features dev-fixtures && cargo test \
  && scripts/sweep-gallery.sh && scripts/check-windows.sh
```

---

## Phase 1 — 归集(sweep):一次把几种币发给同一个人

对照清单里最大的一件:`ToggleMultiToken` / `ToggleAllMultiTokens` /
`SetMultiNetwork` / `ConfirmMultiSelection` 四个事件,和 `multi_select_mode` /
`multi_selected_ids` / `multi_valuable_ids` / `multi_chain_id` / `multi_specs`
五个视图字段,桌面**一个都没读过**。而 DSD1L 那句"发送多种代币"从 spec 021 起就画在
列表底下,是一行居中的灰字 —— **没有监听器**。

### 一个必须留在壳里的标志

核心的 `multi_select_mode` 只在选择被**确认**时才翻,所以"勾选框正在显示吗"在视图里
根本没有对应字段。web 的移植记了同一件事(`live-send.ts` 的 `sweepPicking`),手机
也一样(`TokenSelector.tsx` 的 `sweepActive`)。所以 `send_sweeping` 是页面的:
**哪些币能选、什么算"有价值"、一次归集搬多少,全都还是核心的。**

### 第一次点,决定这是哪条链

一笔批量只能在一条链上。手机用筛选器钉,画稿(SD1b)用**第一次点**钉:第一下命名网络,
之后核心拒绝其他链的每一行。所以行的监听器在 `multi_chain_id` 还是 `None` 时先发
`SetMultiNetwork`,再发 `ToggleMultiToken`。清空选择会解钉,于是"重来一次"不用离开这一屏。

### 别的链的行变灰,不消失

这条是 SD1b 自己的注释,也是这一刀唯一值得单独写测试的规则:**那些币还是这个人的**。
列表在有人勾了一下之后悄悄变短,读起来就是"我的钱不见了"。所以灰而不删,而且灰掉的行
不再可点 —— 核心反正会拒,画成可点是一个它不会兑现的邀请。

"全选有价值的"发的是 `ToggleAllMultiTokens { visible_ids }`:**范围是屏幕上正在显示的**,
而这些里面哪些算有价值,仍然是核心答。

### 零新键

`send.multiSendTitle` / `multiSendChainNotice` / `selectAllValuable` /
`multiSendContinue` 全都在语料里,十五种语言齐全 —— 手机先画的这一屏。

### 测试与实机

一条纯映射测试(勾了什么、灰了哪些、药丸说哪条链、CTA 数几个),视图从真机器取一份再替换
sweep 字段(`wallet::live` 的老办法:手写一个几十个字段的视图等于对它们的不变量瞎猜)。

实机(parallel space 金标 Safe,只有一条链有钱):按"发送多种代币"→ 勾选框和"全选有价值的"
出现;点 xDAI → **Gnosis 的圆标 + "Gnosis selected — a multi-token send stays on one
network, so the others are greyed out"**,行染上强调色,底部变成橙色的 `Send 1 · Gnosis →`。

desktop **342 / 338**(+1 测试),fmt clean,画廊全渲染。
