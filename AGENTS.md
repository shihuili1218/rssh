rssh/
├── FEATURES.md                     # 功能基线
├── index.html                      # 入口 HTML
├── package.json                    # 前端依赖 (Svelte 5 + xterm.js + Tauri API)
├── vite.config.ts                  # Vite 构建配置
├── svelte.config.js
├── tsconfig.json
├── src/                            # 前端 (Svelte + TypeScript)
│   ├── main.ts
│   ├── App.svelte
│   └── styles/global.css
└── src-tauri/                      # Rust 后端
├── Cargo.toml                  # Rust 依赖
├── build.rs
├── tauri.conf.json
├── capabilities/default.json
└── src/
├── main.rs                 # 入口
├── lib.rs                  # 模块声明 + Tauri Builder
├── error.rs                # AppError (thiserror)
├── models.rs               # 领域类型: Profile, Credential, Forward, etc.
├── state.rs                # AppState (Mutex<Connection>)
├── ssh/                    # SSH 核心
│   ├── client.rs           # 连接/认证/shell (todo → russh)
│   ├── auth.rs             # 认证策略
│   ├── sftp.rs             # SFTP 操作 (todo → russh-sftp)
│   ├── forward.rs          # 端口转发 (todo)
│   └── config.rs           # ~/.ssh/config 解析 (已实现)
├── terminal/               # 终端
│   ├── pty.rs              # 本地 PTY (todo → portable-pty)
│   └── recorder.rs         # asciicast v2 录制 (已实现)
├── db/                     # 数据层
│   ├── schema.rs           # DDL + 迁移 (已实现)
│   ├── profile.rs          # Profile CRUD (已实现)
│   ├── credential.rs       # Credential CRUD (已实现)
│   ├── forward.rs          # Forward CRUD (已实现)
│   ├── settings.rs         # Settings KV (已实现)
│   ├── highlight.rs        # 高亮规则 (已实现)
│   └── snippet.rs          # 命令片段 JSON (已实现)
├── sync/
│   └── github.rs           # GitHub 备份 (todo → reqwest)
└── commands/               # Tauri 命令层
├── profile.rs          # Profile/Credential/SSH Config 命令 (已实现)
├── forward.rs          # Forward 命令 (已实现)
└── settings.rs         # Settings/Highlight/Snippet 命令 (已实现)