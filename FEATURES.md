# RSSH 功能清单

从 Flutter 版本提取，作为 Tauri + Rust 重写的需求基线。

---

## 1. SSH 连接

- **认证方式**：密码、私钥(PEM)、键盘交互、无认证
- **ProxyJump**：通过跳板机嵌套 SSH 隧道连接目标主机
- **SSH Config 导入**：解析 `~/.ssh/config`，提取 Host/Hostname/Port/User/IdentityFile/ProxyJump
- **连接后初始命令**：连接成功后自动执行 shell_command

## 2. 终端模拟

- **xterm 终端**：ANSI/VT100 终端模拟，10000 行滚动缓冲
- **Alt-buffer**：支持 vim/less/man 等全屏程序
- **终端搜索**：实时搜索 + 高亮匹配 + 上下导航
- **关键词高亮**：自定义 ANSI 彩色关键词规则（ERROR/WARN/INFO/DEBUG 等，14 种预设颜色）
- **文本选择**：双击选词、三击选行、拖拽选择、Shift+点击扩展
- **终端缩放**：窗口/窗格大小变化时自动调整行列数

## 3. 多窗格分屏

- **水平/垂直分割**：二叉树结构，任意深度嵌套
- **拖拽调整比例**：6px 拖拽条，基于比例的窗格尺寸分配
- **焦点切换**：Cmd+[ / Cmd+] 切换活跃窗格，彩色边框指示
- **独立会话**：每个窗格独立 SSH 连接和终端状态

## 4. SFTP 文件浏览

- **远程目录浏览**：文件夹优先排序 + 字母排序
- **文件上传/下载**：异步传输
- **路径导航栏**：目录层级导航
- **Home 目录自动检测**

## 5. 端口转发

- **本地转发**：LocalPort → RemoteHost:RemotePort
- **远程转发**：RemotePort → LocalHost:LocalPort
- **转发管理**：命名配置、绑定 SSH Profile、独立转发标签页
- **实时统计**：字节流量、活跃连接数、状态跟踪

## 6. 本地终端

- **macOS/Linux**：自动检测 zsh/bash/sh（优先 $SHELL）
- **Windows**：PowerShell 7 / PowerShell 5 / cmd.exe
- **PTY 分配**：伪终端，独立会话
- **OSC 序列**：终端内 UI 交互指令（打开 Profile、打开转发）

## 7. 会话录制与回放

- **asciicast v2 格式**：NDJSON .cast 文件录制
- **可调速回放**：0.5x - 2x 播放速度
- **播放控制**：播放/暂停/停止，事件索引导航
- **ASCII 日志查看**：ANSI 转义序列剥离，回车可视化

## 8. Profile 与凭证管理

- **SSH Profile**：名称、主机、端口、关联凭证、跳板机、初始命令
- **凭证管理**：密码/密钥/无/交互 四种类型，SQLite 存储
- **Profile 管理界面**：增删改查，名称唯一性校验

## 9. 数据持久化

- **SQLite**：`~/.rssh/rssh.db`，v8 schema
- **表**：profiles, credentials, forwards, settings, highlights
- **JSON 文件**：snippets（命令片段）
- **配置导出/导入**：JSON 格式，支持密钥脱敏

## 10. GitHub 同步

- **远程备份**：Token 认证，指定仓库和分支
- **自动提交**：带时间戳，Base64 编码，SHA 跟踪更新

## 11. 命令片段 (Snippets)

- **命名可复用命令**：快捷访问 Cmd+E
- **增删改排序**
- **移动端**：底栏闪电按钮触发

## 12. 主题系统

- **5 套内置主题**：CRT（磷光绿复古）、NEU（拟物）、Dracula、Nord、Catppuccin
- **每套 30+ 色彩 token**：终端色、UI 色、装饰色
- **光标闪烁开关**
- **CRT 特效**：扫描线、复古边框、LED 指示灯

## 13. 窗口控制（macOS）

- **最小化/最大化/关闭**
- **窗口拖拽**
- **适应屏幕布局**
- **置顶模式**
- **新窗口启动并自动连接指定 Profile**

## 14. 键盘快捷键

| 快捷键 | 功能 |
|---|---|
| Cmd+F | 终端搜索 |
| Cmd+E | 命令片段 |
| Cmd+O | SFTP 浏览器 |
| Cmd+\ | 垂直分屏 |
| Cmd+Shift+\ | 水平分屏 |
| Cmd+[ / ] | 切换窗格焦点 |
| Cmd+W | 关闭当前窗格 |

## 15. 移动端适配

- **虚拟键盘**：粘滞修饰键(Ctrl/Shift/Alt)、特殊键序列
- **紧凑键盘布局**：减小高度，图标化按键
- **底栏快捷操作**

## 16. CLI 工具 (sl 命令)

- **sl ls [query]**：列出/搜索 Profile
- **sl open \<name\>**：连接 Profile
- **sl add/edit/rm**：Profile/凭证 CRUD
- **sl push/pull**：GitHub 配置同步

## 17. 设置

- **首页模式**：Profile 列表 / 本地终端
- **SSH 引擎选择**：系统原生 / 纯 Rust
- **录制开关与路径**
- **主题选择**
- **光标闪烁**
- **紧凑键盘**
- **详细连接日志**

## 18. 错误处理

- **连接错误捕获与展示**
- **SFTP 不可用原因提示**
- **跳板机连接失败上报**
- **全局崩溃日志**：`~/.rssh/crash.log`
