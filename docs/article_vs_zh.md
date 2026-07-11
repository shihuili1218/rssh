# 凭什么是 rssh —— 一个不想抢你地盘的 SSH 客户端

判断一个 SSH 客户端是不是好工具，看一件事：

**它有没有想方设法把自己塞进你和服务器之间。**

随手打开市面上几个主流 SSH 客户端，你会撞见一连串荒谬：先要你登录一个云账号；自己维护一套和系统不通的 host key 数据库；要你在每台目标服务器上 source 一个 shell 集成脚本；专有的录制格式、专有的同步、专有的一切。

SSH 协议 1995 年发布。30 年后，连一台服务器，居然要先注册另一个公司的账号、要把私钥上传到别人的服务器、要修改目标机器的 dotfile —— 这件事本身就是这个行业最大的笑话。

rssh 的全部设计，就一句话：

> **工具要服务你已有的工作方式，而不是逼你为工具让步。**

---

## 一张表，看清差异

|  | 主流云同步 SSH 客户端 | 主流开源终端 | 主流命令块终端 | 系统终端 | 老牌商业 SSH | **rssh** |
|---|---------------|---|---|---|---|---|
| 共享 `~/.ssh/known_hosts` | ❌ 自管          | ❌ 自管 | ✅ | ✅ | ❌ 自管 | **✅** |
| CLI + GUI 同源数据 | ❌             | ❌ | ❌ | — | ❌ | **✅ 同一 SQLite** |
| 命令块视觉分组 | ❌             | ❌ | ✅ 需改服务器 | ❌ | ❌ | **✅ 零服务器改动** |
| 内置 AI 排障 | ⚠️ 订阅         | ❌ | ⚠️ 订阅 | ❌ | ❌ | **✅ 自带 + 四道墙** |
| 同步存储位置 | 厂商服务器         | 自建 | 厂商服务器 | — | 厂商服务器 | **你的 GitHub repo** |
| 会话录制格式 | 专有            | — | — | — | 专有 | **asciicast v2** |
| 登录账号 | 必须            | 不需要 | 必须 | — | 注册 | **不需要** |
| 订阅收费 | 月费            | 免费 | 月费 | 免费 | 一次性付费 | **免费 MIT** |
| 移动端 | 有             | ❌ | ❌ | ❌ | ❌ | **Android（iOS 自打包）** |
| 遥测 | 有             | 可关 | 有 | 无 | 有 | **没写过这部分代码** |

（横轴是几类产品的典型代表，不点名 —— 你心里有数。）

---

## 五个根本性差异

### 一、Host Key：用系统的，不另起炉灶

绝大多数 SSH GUI 客户端有一个让人抓狂的设计：自己维护一套 host key 数据库。

意味着什么？意味着你在命令行 `ssh prod` 信任过的主机，到 GUI 里要再确认一次；你在 GUI 里删掉的指纹，命令行 `ssh-keygen -R` 还要再删一次。**同一个事实，两份真理**。

rssh 直接读写 `~/.ssh/known_hosts`。一份数据，两个工具共用。

这不是什么了不起的技术决策，这是**默认就该这样**。其他客户端为什么不这么做？因为如果他们用系统的 known_hosts，你换工具的成本就是零，他们没有了锁定你的钩子。

### 二、CLI 和 GUI：一份数据，两个入口

rssh 的 CLI 不是 GUI 的附庸。**两者读同一个 SQLite**：`~/.rssh/rssh.db`。

```bash
rssh profile list prod            # 模糊搜索
rssh profile open gateway-01      # 直接连，在你自己的终端里
rssh forward open my-tunnel       # 启动命名端口转发
```

意味着：

- 你可以把 `rssh profile open foo` 写进 alias、Makefile、CI 脚本
- 你可以在 GUI 里维护 profile，在 tmux 里使用
- 你不需要为了"图形化管理"维护两份重复的配置

市面上号称有 CLI 的同类产品不少 —— 你试试，要不要登录，要不要 token，能不能拿到 GUI 里建的 profile？

### 三、Command Block：思路对，但代价不该让你扛

有些终端的命令块功能确实好用。你能折叠、能选中、能独立复制每条命令的输出。

但代价是 —— **在服务器上改 shell 集成**。

这在很多场景下根本不可能：

- 公司堡垒机后面的目标机器，你没 sudo
- 一次性救火的客户机器，你不能污染人家 dotfile
- `kubectl exec` 进 pod，根本没你的脚本
- 一万台机器全装一遍？运维只想骂街

rssh 的做法：**完全前端实现**。

每条命令在终端左侧画一道竖向色条，输入和输出共享同色。下一条命令换色（黄金角 HSL 算法保证相邻颜色对比最大）。进入 vim/top/less 全屏程序时色条自动淡出。

**rssh 不知道你在哪台机器上，也不需要知道**。一次连接立刻生效，包括你客户的服务器、别人的堡垒机。

### 四、AI 排障：内置在 I/O 中间，不是网页搬运工

CPU 跑满了、Java 堆爆了、Go 进程吃 8 个 G —— 现在主流的"AI 排障"流程是：

1. 在终端跑命令，肉眼看输出
2. 切到 AI 对话网页，复制粘贴
3. AI 回一条命令，复制回终端
4. 把输出再复制回 AI ……

人变成了 LLM 和终端之间的搬运工。token、密码、内网 IP 直接外发，没人替你脱敏。

SSH 客户端本来就在命令的 I/O 中间，这件事就该它来做。rssh 给 LLM 四个工具：

```
run_command(cmd, explain, side_effect, timeout_s?)
download_file(remote_path, max_mb)
analyze_locally(local_path, task)
load_skill(id)
```

四道硬墙在 Rust 代码里 enforce，**不靠 prompt 自律**：

1. **Shape validator** —— 任何工具调用先过结构校验，prompt 注入也绕不过
2. **你的授权** —— 高风险命令必须人工确认才执行
3. **本地脱敏** —— payload 离机前 token/密码/IP 已经替换为占位符
4. **本地分析窗口** —— 大文件不发给云端，开新窗口用本地 LLM 处理

代码在 `src-tauri/src/ai/`，欢迎审计。

### 五、同步：你的密钥进系统钥匙串，你的配置进你控制的存储

绝大多数同类产品的"同步"，本质是把你的密钥上传到他们的服务器，然后向你保证"端到端加密"。**你怎么验证？**

rssh 不替你保管秘密：

- **密码 / 私钥 passphrase** → macOS Keychain / Windows Credential Manager / Linux Secret Service。你信任你自己的钥匙串，胜过任何第三方软件
- **私钥默认不上传** → 每条凭据独立"是否参与同步"开关。私钥极少变更，用 U 盘、AirDrop 拷一次用十年
- **profile / 转发 / 片段** → 加密后推到**你自己的 GitHub 私有仓库或 WebDAV 服务**。不是 rssh 的服务器 —— **rssh 没有服务器**

加密本身没魔法：Argon2id 用固定参数（19 MiB、2 次迭代、1 lane）派生密钥，再由 ChaCha20-Poly1305 做认证加密。代码在 `src-tauri/src/crypto.rs`，一百行能读完。

```bash
rssh config github set     # 配置 token 和仓库
rssh config github push    # 推送到 GitHub
rssh config github pull    # 从 GitHub 拉取
rssh config webdav set     # 配置 WebDAV 地址
rssh config webdav push    # 推送到 WebDAV
rssh config webdav pull    # 从 WebDAV 拉取
```

底层就是加密数据和 GitHub / WebDAV API。**想换工具就换，数据始终在你控制的存储里**。没有锁定，没有订阅，没有"导出到 CSV"按钮。

---

## 我们刻意不做的事

差异化不仅在做什么，更在不做什么：

- **没有注册登录** —— SSH 客户端是个跑在你机器上的程序，不是 SaaS
- **没有云端服务器** —— 我们维护一个云服务，你信任成本就上去了
- **没有订阅** —— 不是商业模式问题，是工程默认值
- **没有遥测开关** —— 因为压根没写过这部分代码
- **没有插件市场** —— 你想要的功能，要么内置，要么不存在
- **没有独立 AI 对话窗口** —— AI 在你 SSH 会话里做事，不是另一个独立产品

---

## 三个问题，决定你要不要用 rssh

不要看营销，问自己三个问题：

**1. 你是不是经常用命令行 ssh？**

是 → rssh 共享 known_hosts 和系统钥匙串，零冲突
否 → 用什么都行

**2. 你介不介意把私钥上传到第三方？**

介意 → rssh 不替你保管，密钥进系统钥匙串
不介意 → 市面上"无脑"体验的付费产品有的是

**3. 你需不需要让 AI 帮你排障，但又不想成为搬运工？**

需要 → rssh 内置 AI + 四道安全墙
不需要 → 一个普通终端就够了

---

## 下载

[Releases](https://github.com/shihuili1218/rssh/releases) 提供 macOS（Intel + Apple Silicon）、Windows、Linux（deb/rpm/AppImage）、Android 安装包。

MIT 协议，无登录，无订阅，无广告，无遥测。

---

**一句话总结**：rssh 不是又一个想替代 SSH 的 SaaS，它是个想让你忘记它存在的 SSH 客户端。
