# RSSH

[English](README.md) | [中文](README_zh.md)

带桌面 GUI 和内置 CLI 的 SSH 连接管理器。

macOS / Windows / Linux / Android。

> 设计理念：[为什么是 RSSH？—— 天生的 AI 运维助手](docs/article_zh.md) ([English](docs/article_en.md))

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/shihuili1218/rssh)

## 亮点

- **AI 排查** —— LLM 驱动的通用运维问题定位；每次工具调用都过 shape validator、你的授权、本地脱敏三道关卡，payload 离机前已清洗
- **命令块色条** —— 零远端依赖，终端命令块自动按颜色分组
- **CLI 优先** —— CLI 与 GUI 共享同一个数据库，任意终端 `rssh open prod`
- **安全与同步** —— 密钥进系统钥匙串，按凭据控制同步范围，加密备份到你自己的 GitHub 仓库

## 功能

- **SSH** —— 密码、私钥、键盘交互、跳板机（ProxyJump）
- **终端** —— xterm 仿真、10 000 行回滚、关键词高亮、搜索
- **SFTP** —— 远程文件浏览、上传/下载
- **端口转发** —— 本地和远程，命名配置，实时流量统计
- **本地终端** —— 自动识别 zsh/bash/PowerShell
- **会话录制** —— asciicast v2 格式，变速回放
- **Profile 与凭据** —— SQLite 存储，可从 `~/.ssh/config` 导入
- **同步** —— 加密导出/导入，GitHub 备份
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
