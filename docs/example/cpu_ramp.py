nohup python3 -c "import multiprocessing as m,time,itertools
for i in itertools.count(1):
    m.Process(target=lambda:exec('while 1:pass'),daemon=True).start()
    print(f'workers={i}',flush=True);time.sleep(5)" > /tmp/cpu_ramp.log 2>&1 &
echo $! > /tmp/cpu_ramp.pid

pkill -f "python3 -c import multiprocessing as m,time,itertools"