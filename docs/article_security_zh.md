# rssh 不替你保管秘密，但实现了多端同步

## 行业的"同步"是个谎言

打开同类 SSH 客户端的官网，"Cloud Sync" / "Settings Sync" / "Workspace Sync" 几乎是标配功能。点进去看说明：

> 您的凭据通过端到端加密同步到我们的服务器。

这句话翻译成工程语言是：

1. 你的私钥、密码、SSH key passphrase 上传到了厂商的数据库
2. 厂商**声称**它是端到端加密的，但密钥派生过程在他们的客户端代码里
3. 你**没办法验证**：客户端是闭源二进制，或者源码开放但 server 端的存储格式你看不到
4. 一旦厂商被攻破（Termius 2020 / LastPass 2022 / 1Password 历次警告），全网用户的 SSH key 一起裸奔

订阅费在替你买什么？买一个**额外的攻击面**。本来你的私钥只放在自己的 `~/.ssh/`，现在多了一份在厂商的 S3 桶里。

rssh 拒绝这条路。但同时，**多端同步本身是真需求** —— 工作机、家里 Mac、移动端 Android，profile 总得能一处编辑处处可用。

## 设计取舍：把"保管"和"同步"拆开

行业的隐含假设是：**要同步，就必须有一个中心节点保管你的数据**。这个假设是错的。

rssh 把"保管"和"同步"明确拆成两件事：

| 数据类型 | 保管在哪 | 怎么同步 |
|---|---|---|
| 私钥 / 密码 / passphrase | 你的操作系统 keychain | 默认**不同步** |
| AI API key / GitHub token | 你的操作系统 keychain | **永不离开本机** |
| profile / 转发规则 / 命令片段 / skill | 你的 SQLite | 加密后推到**你自己的** GitHub repo |
| host key | 标准 `~/.ssh/known_hosts` | 不归 rssh 管，ssh 自己的事 |

没有 rssh 服务器，也没有"账号"。同步靠 GitHub —— 不是因为 GitHub 多安全，是因为 **GitHub 已经是你的基础设施**，你已经信任它存了你的源码。再加一份加密的 240KB JSON，不增加新的攻击面。

## 第一层：本地秘密 → OS Keychain

`src-tauri/src/secret/mod.rs:33` 的 `SecretStore` trait 是所有秘密的唯一入口：

```
macOS    → Keychain（apple-native，硬件辅助）
Windows  → Credential Manager（DPAPI 用户上下文）
Linux    → Secret Service（D-Bus，gnome-keyring / KWallet）
兜底     → SQLite secrets 表（仅 keychain 真不可用时）
```

服务名固定 `"rssh"`，命名规则 `cred:<credential_id>:secret`。启动时做一次写/读/删探测（`src-tauri/src/secret/keyring_store.rs:11`），只有探测通过才用 keychain，没有 "为方便起见也写一份到 DB" 的后门。

**关键设计**：rssh 进程本身不持有长期密钥。每次需要私钥就向 keychain 要一次，用完释放。passphrase 缓存用 `zeroize::Zeroizing<String>` 包装（`src-tauri/src/ssh/auth.rs:59`），`Drop` 时显式把内存抹零 —— 不靠 Rust 默认 drop 语义。

**为什么这条重要**：你信任你自己的 keychain，胜过信任任何第三方软件。这个假设是合理的 —— keychain 是操作系统厂商提供的、与硬件 enclave 绑定、被亿万用户审视过的成熟组件。让 rssh 在它之上再造一层加密，是 NIH（Not Invented Here），不是安全。

## 第二层：每条凭据可选是否参与同步

`src-tauri/src/commands/sync.rs:135` 的 `github_push` 关键三行：

```rust
for c in credentials.iter_mut() {
    if !c.save_to_remote {
        c.secret = None;
    }
}
```

**每条凭据有独立的 `save_to_remote` 开关**。默认 false。

- 私钥这种东西本就极少变更 —— 用 U 盘、AirDrop、`scp` 在两台设备之间拷一次能用十年
- 把它推到云端换"方便"，是用安全换懒惰
- 但你愿意推也可以推 —— 旋钮在你手里，不是 rssh 替你做决定

被排除的凭据，元数据（名称、类型、用户名）仍同步，**只清空 secret 字段**。这样 pull 回来后，profile 还是知道"这条登录要用名为 prod-key 的凭据"，只是本地需要从 keychain 里补上 secret —— 或者干脆在新机器上重新 `scp` 一份私钥进 keychain。

## 第三层：加密格式 —— 100 行可审计

很多产品的"端到端加密"是个黑盒。rssh 的整个加密实现在 `src-tauri/src/crypto.rs`，**100 行能读完，欢迎审计**。

**Wire format（v2）**：

```
base64( version[1] || salt[16] || nonce[12] || ciphertext_with_tag )
```

**算法选择**：

| 组件 | 选型 | 参数 |
|---|---|---|
| KDF | Argon2id | 19 MiB / 2 iter / 1 lane（OWASP 2024 基线） |
| AEAD | ChaCha20-Poly1305 | key 32B / nonce 12B / tag 16B |
| 随机源 | `getrandom` | 操作系统 CSPRNG |

**为什么这些选择**：

1. **Argon2id 不是 PBKDF2/scrypt** —— Argon2 是 2015 年密码哈希竞赛冠军，OWASP 现役推荐。19 MiB 内存成本让 GPU 暴力破解从 "便宜" 变 "贵"
2. **ChaCha20-Poly1305 不是 AES-GCM** —— ChaCha 在没有 AES-NI 指令集的设备（老 Linux 服务器、移动端）性能更稳，没有 nonce 重用灾难
3. **AEAD 自带认证** —— 篡改 ciphertext / nonce / salt 任何一字节，解密报 `crypto_password_or_corrupted`，不会静默给你一段垃圾
4. **参数钉死成常量，不用 `Argon2::default()`** —— 默认值跨 crate 版本会漂移，同一密码同一 salt 在不同 rssh 版本派生出不同 key = 旧备份解不开。这是 v1（自己手搓 SHA-256 + 异或）被废弃换 v2 的教训
5. **第一字节版本号** —— 未来要升参数就 v3，老 v2 备份仍能被识别（即便决定不再支持解码，至少能报"老版本"而不是"密码错"）

测试覆盖（`src-tauri/src/crypto.rs:113-228`）包括：roundtrip、Unicode、空 payload、密码错、ciphertext 篡改、nonce 篡改、salt 篡改、非法 base64、长度不足、版本不支持、两次加密产出不同 blob。每一条都是一类已知的密码学失败模式。

## 第四层：推送通道 —— 你的 GitHub repo，不是 rssh 的服务器

`src-tauri/src/sync/github.rs` 总共 135 行。所有逻辑就是：

```
PUT https://api.github.com/repos/<your>/<repo>/contents/rssh_backup.json
  Authorization: Bearer <your_github_token>
  body: { content: base64(encrypted_blob) }
```

加密 blob → GitHub Contents API → 落到**你自己的私有 repo**。GitHub 看到的是 base64 之后的二进制，没有密钥，没法解。

**为什么是 GitHub 而不是 S3 / Dropbox / iCloud**：

- 工程师本来就有 GitHub 账号，本来就有 PAT（personal access token），不增加新基础设施
- 私有 repo 自带版本历史 —— 误推坏配置可以回退到上一个 commit
- GitHub API 稳定，30 年内不太可能消失
- repo 是**你的**资产，rssh 哪天不维护了，配置文件还在你账号里

**没有锁定**：

```
rssh config github push    # 推到 GitHub
rssh config github pull    # 从 GitHub 拉取
```

底层就是 base64 + GitHub API。想换工具？拿走 `rssh_backup.json` 自己写解码就行 —— wire format 在 `crypto.rs` 开头注释里。没有"导出到 CSV"按钮，因为你的数据本来就在你的 repo 里。

## 拉取语义：事务性全量替换

`src-tauri/src/sync/config.rs:196` 的 `replace_import` 处理 pull：

```rust
// (1) 先 parse 全部条目，任何 parse 失败 → 早 fail，不动 DB
// (2) DB 事务：clear + insert 整体原子，失败回滚
// (3) DB commit 后才动 SecretStore（非事务），先删被淘汰的旧 secret，再写新 secret
```

**为什么先 parse 再事务**：parse 是廉价的，事务期间不能长。如果 parse 失败，本地 DB 完全不动。

**为什么 SecretStore 在事务外**：keychain 不支持 SQL 事务。所以策略是 —— DB 先成功（说明新配置完整），再去同步 secrets；DB 失败时本地 secrets 一字不动，与 DB 一起完整回滚到旧状态。这避免了 "DB 回滚了但密码已经被改了" 的不一致。

**为什么先抓 old_cred_ids 快照再进事务**：事务后表已经清空，再 list 就是新 ids 了，没法知道哪些旧 cred 该清 secret。快照 + 集合差 = 删除被淘汰的 secret。这是数据迁移的标准模式，但很多产品在这一步漏处理就导致 keychain 里残留无主条目。

## 不做的事

列一遍 rssh **拒绝实现**的功能比列实现了什么更说明问题：

- ❌ 不提供 "rssh 账号" —— 没有账号就没有 server-side 数据库被脱库
- ❌ 不做"私钥自动云备份" —— 私钥要么你显式 opt-in 推（`save_to_remote = true`），要么自己拷
- ❌ 不在 keychain 之外再造一层加密 —— 信任你的 OS 厂商，不要 NIH
- ❌ 不接受弱密码 KDF —— Argon2id 参数钉死在 OWASP 基线
- ❌ 不沉默篡改 —— AEAD tag 校验失败立即报错，绝不返回半截明文
- ❌ 不收订阅费 —— 没有可持续盈利的同步服务 = 没有动机偷偷加 telemetry

## 一句话设计哲学

**同步不需要中心节点**。

你的秘密在你的 keychain。你的配置在你的 GitHub。你的 token 在你的本机。rssh 是一个程序，不是一个 SaaS —— 它跑在你的笔记本上，做完事退出，没有"我们的服务器"。

订阅费替你买的是攻击面。rssh 不收订阅费，因为它压根没有那个攻击面可以替你管。
