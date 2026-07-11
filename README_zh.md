# RSSH

[English](README.md) | [中文](README_zh.md)

**为 AI 运维而生的 SSH 客户端。**

> 连上服务器，直接问"磁盘怎么满了"——AI 提议命令、标注副作用，你点同意它才在终端里执行；敏感信息离机前本地脱敏。
> 
> 桌面 · 手机 · JetBrains · 命令行，一套数据通用。

[![Release](https://img.shields.io/github/v/release/shihuili1218/rssh)](https://github.com/shihuili1218/rssh/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/shihuili1218/rssh/total)](https://github.com/shihuili1218/rssh/releases)
![Platforms](https://img.shields.io/badge/macOS%20·%20Windows%20·%20Linux%20·%20Android-555)
[![License](https://img.shields.io/github/license/shihuili1218/rssh)](LICENSE)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/shihuili1218/rssh)

<p align="center">
  <img src="docs/img_local.png" alt="RSSH —— 问一句，AI 提议命令，你点同意才执行" width="820">
</p>

<p align="center"><b><a href="https://github.com/shihuili1218/rssh/releases">⬇️ 下载最新版</a></b> &nbsp;·&nbsp; <a href="docs/article_zh.md">为什么是 RSSH？</a></p>

---

## 为什么选 RSSH

### 🤖 AI 排障
不是又一个聊天框。它读终端里**真实发生**的事，提议**只读优先**的命令，每条都标注副作用、要你点「执行」才会跑。payload 离机前过 shape 校验 + 本地脱敏——你的密钥、内网地址不会原样发出去。

![AI 排障面板：AI 提议命令，等你批准](docs/welcome-ai.gif)

### 🎨 彩色命令块
每条命令和它的输出自动成块、左侧按色分隔。上千行滚屏里，一眼找到上一条命令的输出在哪儿。**纯本地渲染**，零远端依赖、不在服务器上装任何 agent。

![彩色命令块：左侧分色定位](docs/welcome-blocks.gif)

### ⌨️ 一处配置，处处可用
`rssh open prod` 从任意终端直接拉起会话——CLI 与 GUI 共用同一个 SQLite 库。同一套主机和密钥，还能跑在**手机**和 **JetBrains** 工具窗口里。

![CLI 直连：rssh open prod](docs/welcome-cli.gif)

---

## 功能

- **SSH** —— 密码、私钥、SSH Agent/Pageant、键盘交互、跳板机（ProxyJump）
- **Telnet** —— 保存连接配置、回显协商、行规程设置、expect/send 登录脚本
- **串口控制台（桌面端）** —— 保存 UART 配置、文本/Hex 模式、流控、登录脚本、DTR/RTS/Break 控制
- **动态发现（Docker/K8S）** —— 通过本机 CLI context 发现 Docker 容器和运行中的 Kubernetes Pod，直接打开临时 exec 终端
- **终端** —— xterm 仿真、10 000 行回滚、可折叠彩色命令块、正则高亮、搜索
- **多会话工作台（桌面端）** —— 实时预览已连接终端，并向选中的会话广播内容
- **SFTP** —— 远程文件浏览和上传/下载
- **端口转发** —— 本地、远程和动态（SOCKS5），命名配置，实时流量统计
- **本地终端（桌面端）** —— 自动识别 zsh/bash/PowerShell
- **会话录制** —— asciicast v2 格式，变速回放
- **连接与凭据** —— SQLite 存储，可从 `~/.ssh/config` 导入
- **安全与同步** —— Secret 使用 ChaCha20-Poly1305 本地加密，系统可用时由钥匙串保管主密钥；可选择远程同步范围，并加密备份到你自己的 GitHub 仓库或 WebDAV 服务器
- **片段** —— 可复用命令快捷键（Cmd+E）
- **移动端** —— 虚拟键盘栏（Ctrl/Alt/方向键/Tab/Esc）、安全区、栈式导航
- **IDE 插件** —— 在 JetBrains IDE 的工具窗口里运行 RSSH（共享数据目录）

## 安装

从 [Releases](https://github.com/shihuili1218/rssh/releases) 下载：

| 平台                  | 文件                                   | 备注              |
|---------------------|--------------------------------------|-----------------|
| macOS Apple Silicon | `rssh-{ver}-macos-aarch64.dmg`       |                 |
| macOS Intel         | `rssh-{ver}-macos-x86_64.dmg`        |                 |
| Linux (deb)         | `rssh-{ver}-linux-x86_64.deb`        | Debian/Ubuntu   |
| Linux (rpm)         | `rssh-{ver}-linux-x86_64.rpm`        | Fedora/RHEL     |
| Linux (AppImage)    | `rssh-{ver}-linux-x86_64.AppImage`   | 任意发行版           |
| Windows             | `rssh-{ver}-windows-x86_64.msi`      | 静默安装：`msiexec /i` |
| Windows             | `rssh-{ver}-windows-x86_64-setup.exe` | 图形安装器           |
| Windows             | `rssh-{ver}-windows-x86_64-portable.zip` | 免安装 GUI + CLI  |
| Android             | `rssh-{ver}-android-universal.apk`   |                 |
| iOS                 |                                      | 无官方构建；需自行打包      |

### IntelliJ / JetBrains 插件

在 JetBrains IDE 的工具窗口里运行完整 RSSH —— 与桌面版共享同一套主机、密钥、设置
（共享 `~/.rssh`）。每个 zip 内置 headless `rssh-server`，自包含、按平台区分：

| 平台                  | 文件                                              |
|---------------------|--------------------------------------------------|
| macOS Apple Silicon | `rssh-{ver}-macos-aarch64-jetbrains-plugin.zip`  |
| macOS Intel         | `rssh-{ver}-macos-x86_64-jetbrains-plugin.zip`   |
| Linux               | `rssh-{ver}-linux-x86_64-jetbrains-plugin.zip`   |
| Windows             | `rssh-{ver}-windows-x86_64-jetbrains-plugin.zip` |

安装：**Settings → Plugins → ⚙ → Install Plugin from Disk…**，选对应平台的 zip 后重启。
打开底部 **RSSH** 工具窗口即可使用；标题栏的 ✕ 停止内置 server。

## 开发

参见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 协议

MIT

## 友情链接

- [LINUX DO - 新的理想型社区](https://linux.do/)
