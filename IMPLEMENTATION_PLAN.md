# Telnet 支持实施计划

设计基调：telnet 是第四种 transport，骨架 1:1 抄串口（models → db → commands → state → TerminalPane → AI 面板），
不做统一 transport trait 抽象。AI 面板走串口同款"手动提交输出"路径（无 shell、无 sentinel、无退出码），
但 prompt 引导文案面向网络设备 CLI 而非 UART。

## 第 1 阶段：后端 telnet 传输层
**目标**: `terminal/telnet.rs`（纯函数 IAC 协商状态机 + TcpStream 读线程）、`commands/telnet.rs`
（telnet_open/write/resize/close）、state/lifecycle/lib.rs 接线
**成功标准**: cargo test 绿；协商状态机纯函数可测（ECHO/SGA/NAWS/TTYPE/BINARY、IAC 转义、拒绝未知选项、防协商环）
**测试**: 状态机单测 + 127.0.0.1 loopback 集成测试（脚本化 telnet server）
**状态**: 完成

## 第 2 阶段：TelnetProfile 持久化
**目标**: models.rs `TelnetProfile`（host/port + input_newline=crlf/output_newline/local_echo/backspace/login_script/group_id，
不带串口专属的 baud/hex/slow_send/xany）、schema.rs 建表、`db/telnet_profile.rs` CRUD、
commands CRUD、sync/config.rs 导入导出
**成功标准**: cargo test 绿，含 db roundtrip 测试；sync 导出含 telnet_profiles
**测试**: 仿 db/serial_profile.rs 测试集
**状态**: 完成

## 第 3 阶段：前端接入
**目标**: TabType 加 "telnet"、TerminalPane TRANSPORT 表 + open 分支 + 复用串口行变换管线（EOL/backspace/login_script）、
connectTelnetProfile、TelnetProfileManager/Editor（抄 Serial 兄弟组件）、HomeScreen/SettingsLayout/AppShell 挂点、i18n en/zh
**成功标准**: npm run check / vitest 绿；能开 telnet tab 连上真实服务
**测试**: 现有 vitest 套件 + 手工连接验证
**状态**: 完成

## 第 4 阶段：AI 面板
**目标**: ShellKind::Telnet（无 sentinel + 网络设备 prompt 文案）、AiTarget::Telnet、AiTargetKind "telnet"、
store TRANSPORT/dropEcho/手动 submit、CommandConfirmDialog 提交输出按钮、会话 scope "telnet:<host>"
**成功标准**: cargo test + vitest 绿；telnet tab 里 AI run_command 走手动提交流程
**测试**: shell.rs telnet 单测（仿 serial 组）+ 前端类型收窄编译检查
**状态**: 完成

## 第 5 阶段：headless server + 收尾
**目标**: server.rs ws 方法（telnet_open/write/resize/close）、roadmap.md 勾掉、全量测试、scoped fmt、自审 + codex 二审
**成功标准**: cargo test --all-features + vitest 全绿
**测试**: 全量套件
**状态**: 完成
