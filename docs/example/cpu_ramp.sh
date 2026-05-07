#!/usr/bin/env bash
# AI 排障演示：每 5 秒 fork 一个吃满一核的 Python 进程，CPU 占用阶梯式上升。
# 启动：
nohup python3 -c "import multiprocessing as m,time,itertools
for i in itertools.count(1):
    m.Process(target=lambda:exec('while 1:pass'),daemon=True).start()
    print(f'workers={i}',flush=True);time.sleep(5)" > /tmp/cpu_ramp.log 2>&1 &
echo $! > /tmp/cpu_ramp.pid

# 干掉所有子进程：
pkill -f "python3 -c import multiprocessing as m,time,itertools"