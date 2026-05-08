- SFTP 上传/下载：增加下载页面（后台任务、历史记录、失败按钮重试）
- slip window
- 补齐cli功能（group...）cli-first
- Host key / known_hosts 可视化
- rssh status CLI 子命令 — 列当前活跃 SSH session / forward / SFTP
- 拆分线程，现在所有会话的所有操作都在一个线程上执行。改成线程池（注意： SFTP 重连场景、后续 Handle 操作），暂时没有瓶颈

好好扫描一下所有rust代码，相当于一次完整review和refactor，找出不好设计/数据结构错误/错误实现/badsmell，我需要真实的问题，而不是为了找出问题而找出问题                    