- 补齐cli功能（group...）cli-first，rssh status CLI 子命令 — 列当前活跃 SSH session / forward / SFTP
- Host key / known_hosts 可视化
- 拆分线程，现在所有会话的所有操作都在一个线程上执行。改成线程池（注意： SFTP 重连场景、后续 Handle 操作），暂时没有瓶颈
- 命令片段搜索最近命令
- 无活动锁定密码
- 增加只读
- 隐藏标题栏
- home搜索框，改成快速连接 ？
- 导入阿里云、aws账号资源
- AI 上下文管理，压缩、历史记忆(rag?)
- 外观字体只能控制终端，最好要控制全局字体（AI）❌
- AI 导入～/CLAUDE.md
- 统一图标：自绘 SVG 替换 emoji（MobileKeybar 的 ⚡/📁、SftpBrowser 的 📁🔗📄），落实 AGENT.md「图标自己画，别用emoji」
- 接入cloudflare https://linux.do/t/topic/2487408/74
- https://linux.do/t/topic/2487408/70
- cat profile.html ls折叠/展开，会出现很多clear不掉的行，调整size后clear会清理 ???

![Stars](https://img.shields.io/github/stars/shihuili1218/rssh)
![Forks](https://img.shields.io/github/forks/shihuili1218/rssh)
![Watchers](https://img.shields.io/github/watchers/shihuili1218/rssh)
![Contributors](https://img.shields.io/github/contributors/shihuili1218/rssh)
![Open Issues](https://img.shields.io/github/issues/shihuili1218/rssh)
![Closed Issues](https://img.shields.io/github/issues-closed/shihuili1218/rssh)
![Open PRs](https://img.shields.io/github/issues-pr/shihuili1218/rssh)
![Last Commit](https://img.shields.io/github/last-commit/shihuili1218/rssh)
![Commit Activity](https://img.shields.io/github/commit-activity/m/shihuili1218/rssh)
![Commits Since Release](https://img.shields.io/github/commits-since/shihuili1218/rssh/latest)
![Total Downloads](https://img.shields.io/github/downloads/shihuili1218/rssh/total)
![Latest Downloads](https://img.shields.io/github/downloads/shihuili1218/rssh/latest/total)
![Release](https://img.shields.io/github/v/release/shihuili1218/rssh)
![Release Date](https://img.shields.io/github/release-date/shihuili1218/rssh)