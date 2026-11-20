#!/usr/bin/env bash
# Usage: tools/ci/load_probe.sh
# One line on how hard a verify part used its runner, for choosing runner sizes (#18843): its cores, the load averages,
# how busy the cores were on average since the machine started (a Blacksmith runner starts for one job), memory, and
# the job's seconds so far. It only prints: no value it cannot read stops it, and the steps add `|| echo` as well.
cores=$(nproc 2>/dev/null) || cores='?'
load=$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null) || load='?'
# /proc/stat's first line: user nice system idle iowait irq softirq steal, in ticks of all cores since boot.
busy=$(awk '/^cpu /{b=$2+$3+$4+$7+$8+$9; t=b+$5+$6; if (t > 0) printf "%d%%", 100 * b / t; exit}' /proc/stat 2>/dev/null) ||
  busy='?'
up=$(cut -d' ' -f1 /proc/uptime 2>/dev/null) || up='?'
memory=$(awk '/^MemTotal:/{t=$2} /^MemAvailable:/{a=$2} END{if (t) printf "%d of %d MB", (t-a)/1024, t/1024}' \
  /proc/meminfo 2>/dev/null) || memory='?'
# The most memory the job's processes held at once (cgroup v2's memory.peak), where the kernel keeps it.
peak='unknown'
group=$(sed -n 's/^0:://p' /proc/self/cgroup 2>/dev/null)
if [ -n "$group" ] && [ -r "/sys/fs/cgroup$group/memory.peak" ]; then
  peak="$(( $(cat "/sys/fs/cgroup$group/memory.peak") / 1048576 )) MB"
fi
# The runner's worker process starts with the job: its age is the job's time so far.
job='?'
pid=$$
while [ -n "$pid" ] && [ "$pid" -gt 1 ] 2>/dev/null; do
  if [ "$(ps -o comm= -p "$pid" 2>/dev/null)" = Runner.Worker ]; then
    job=$(ps -o etimes= -p "$pid" 2>/dev/null | tr -d ' ')
    break
  fi
  pid=$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')
done
echo "load probe: ${GITHUB_JOB:-?} on ${cores:-?} cores: load ${load:-?} (1, 5, 15 min), cores ${busy:-?} busy since boot" \
  "${up:-?} s ago, memory ${memory:-?} in use now, peak ${peak}, job ${job:-?} s"
exit 0
