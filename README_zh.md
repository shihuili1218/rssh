# RSSH

[English](README.md) | [中文](README_zh.md)

**为 AI 运维而生的 SSH 客户端。**

> 连上服务器，直接问"磁盘怎么满了"——AI 提议命令、标注副作用，你点同意它才在终端里执行；敏感信息离机前本地脱敏。
> 桌面 · 手机 · JetBrains · 命令行，一套数据通用。

[![Release](https://img.shields.io/github/v/release/shihuili1218/rssh)](../../releases/latest)
[![Downloads](https://img.shields.io/github/downloads/shihuili1218/rssh/total)](../../releases)
![Platforms](https://img.shields.io/badge/macOS%20·%20Windows%20·%20Linux%20·%20Android-555)
[![License](https://img.shields.io/github/license/shihuili1218/rssh)](LICENSE)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/shihuili1218/rssh)

<p align="center">
  <img src="docs/welcome-ai.gif" alt="RSSH —— 问一句，AI 提议命令，你点同意才执行" width="820">
</p>

<p align="center"><b><a href="../../releases/latest">⬇️ 下载最新版</a></b> &nbsp;·&nbsp; <a href="docs/article_zh.md">为什么是 RSSH？</a></p>

---

## 为什么选 RSSH

### 🤖 AI 排障，你始终在回路里
不是又一个聊天框。它读终端里**真实发生**的事，提议**只读优先**的命令，每条都标注副作用、要你点「执行」才会跑。payload 离机前过 shape 校验 + 本地脱敏——你的密钥、内网地址不会原样发出去。

![AI 排障面板：AI 提议命令，等你批准](docs/img_ai_panel.png)

### 🎨 彩色命令块
每条命令和它的输出自动成块、左侧按色分隔。上千行滚屏里，一眼找到上一条命令的输出在哪儿。**纯本地渲染**，零远端依赖、不在服务器上装任何 agent。

![彩色命令块：左侧分色定位](docs/img.png)

### ⌨️ 一处配置，处处可用
`rssh open prod` 从任意终端直接拉起会话——CLI 与 GUI 共用同一个 SQLite 库。同一套主机和密钥，还能跑在**手机**和 **JetBrains** 工具窗口里。

![CLI 直连：rssh open prod](docs/welcome-cli.gif)

---

## 功能

- **SSH** —— 密码、私钥、键盘交互、跳板机（ProxyJump）
- **终端** —— xterm 仿真、10 000 行回滚、关键词高亮、搜索
- **SFTP** —— 远程文件浏览、上传/下载
- **端口转发** —— 本地和远程，命名配置，实时流量统计
- **本地终端** —— 自动识别 zsh/bash/PowerShell
- **会话录制** —— asciicast v2 格式，变速回放
- **Profile 与凭据** —— SQLite 存储，可从 `~/.ssh/config` 导入
- **安全与同步** —— 密钥进系统钥匙串，按凭据控制同步范围，加密备份到你自己的 GitHub 仓库
- **片段** —— 可复用命令快捷键（Cmd+E）
- **移动端** —— 虚拟键盘栏（Ctrl/Alt/方向键/Tab/Esc）、安全区、栈式导航
- **IDE 插件** —— 在 JetBrains IDE 的工具窗口里运行 RSSH（共享数据目录）

## 安装

从 [Releases](../../releases) 下载：

| 平台                  | 文件                                   | 备注              |
|---------------------|--------------------------------------|-----------------|
| macOS Apple Silicon | `rssh-{ver}-macos-aarch64.dmg`       |                 |
| macOS Intel         | `rssh-{ver}-macos-x86_64.dmg`        |                 |
| Linux (deb)         | `rssh-{ver}-linux-x86_64.deb`        | Debian/Ubuntu   |
| Linux (rpm)         | `rssh-{ver}-linux-x86_64.rpm`        | Fedora/RHEL     |
| Linux (AppImage)    | `rssh-{ver}-linux-x86_64.AppImage`   | 任意发行版           |
| Windows             | `rssh-{ver}-windows-x86_64.msi`      | 静默安装：`msiexec /i` |
| Windows             | `rssh-{ver}-windows-x86_64-setup.exe` | 图形安装器           |
| Android             | `rssh-{ver}-android-universal.apk`   |                 |
| iOS                 |                                      | 没有开发者账号，自行打包    |

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
