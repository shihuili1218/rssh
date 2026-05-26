- slip window
- 补齐cli功能（group...）cli-first
- Host key / known_hosts 可视化
- rssh status CLI 子命令 — 列当前活跃 SSH session / forward / SFTP
- 拆分线程，现在所有会话的所有操作都在一个线程上执行。改成线程池（注意： SFTP 重连场景、后续 Handle 操作），暂时没有瓶颈
- ai/sanitize.rs:33-52 脱敏规则覆盖不全，当前覆盖：内网 IP / Bearer / sk-* / JWT / hex(≥32)，漏：AWS access key (AKIA[0-9A-Z]{16})
- ai 增加正则脱敏，增加命令黑名单
- lszrz ❌
- 远程html，转发本地打开

好好扫描一下所有rust代码，相当于一次完整review和refactor，找出不好设计/数据结构错误/错误实现/badsmell，我需要真实的问题，而不是为了找出问题而找出问题

