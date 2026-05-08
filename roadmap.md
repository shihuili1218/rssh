- SFTP 上传/下载：增加下载页面（后台任务、历史记录、失败按钮重试）
- slip window
- 补齐cli功能（group...）cli-first
- Host key / known_hosts 可视化
- rssh status CLI 子命令 — 列当前活跃 SSH session / forward / SFTP
- 拆分线程，现在所有会话的所有操作都在一个线程上执行。改成线程池（注意： SFTP 重连场景、后续 Handle 操作），暂时没有瓶颈



🟡 该改但不紧急（设计错误 / 资源管理 / smell）

锁与 unwrap

- ssh/client.rs:514 *mismatch.lock().unwrap()：用 locked() helper 替换。实际不会触发（mismatch 临界区无 panic），所以是防御性，不是活 bug。
- ssh/forward.rs:172 同上。

等待者 / 资源泄漏

- ssh/client.rs:222-223 prompt_oneshot 用 insert 覆盖旧 sender。注释自己说"理论不会发生"。要么改成"已存在则 return error"，要么删注释把它当真 invariant 检测。Linus              
  rule：能消除特殊情况就消除。
- commands/session.rs:197-246：tab 中途关闭时 auth_waiters / passphrase_waiters / host_key_waiters 不清。Receiver 永远 hang，对应 SSH session 也吊着。ssh_disconnect             
  路径要扫这三张表按 tab_id 清。
- commands/sftp.rs:170/247/268：register_cancel_flag → streaming → unregister_cancel_flag。如果中间 panic，flag 漏。改成 RAII guard（struct CancelGuard impl Drop 自动
  unregister）。

事件命名违反 R1

- ssh/sftp.rs:251, 305 emit "sftp:progress" 不带 transfer_id。多并发传输互串。改 format!("sftp:progress:{transfer_id}")。

事务缺失

- db/group.rs:59-66 删 group + update profiles 两条语句无 transaction。
- commands/sync.rs:63-171 apply_import 同样。

结构 smell

- bin/rssh.rs 1449 行单文件，cmd_open_ssh 与 cmd_open_fwd（L334-422 vs 424-513）有 ~85 行重复 bastion 链 + 私钥临时文件构造。find_profile_id / find_credential_id /              
  find_forward_id（L907-929）三份相同模板。该拆 commands/{open,add,edit,rm,config}.rs + helpers/{tui,ssh_builder}.rs。
- ssh/client.rs 1450 行单文件：连接、认证、prompt 链、sftp 桥、disconnect 全在一起。至少拆出 auth.rs（私钥 / 密码 / interactive）和 prompt.rs（passphrase / host_key 通用 oneshot
  模式）。

其他

- lib.rs:47-51：每次启动遍历 credentials 删旧 passphrase keychain key。是一次性 migration，应该写入 schema_version 标志改成只跑一次。现在每次启动都打 keychain N 次。
- state.rs:36 passphrase_cache：HashMap<String, String> 进程内明文。注释说"绝不落盘"——OK，但 String drop 不清零。考虑 Zeroizing<String>。真实风险低（要本机内存
  dump），列出来供权衡。
- ai/session.rs handle_run_command tool_payload：tool result 进 history 时是否经过 redact？若否，结合 🔴 #1 一并修。



