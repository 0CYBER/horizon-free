#!/usr/bin/env python3
"""Pauses the Horizon CI runner (tools/ci/linux-runner.sh) while the maintainer plays a game on this machine.

A game is running when any of these holds:
  - fleet-load says so (~/fleet/load: Steam, gamescope, Wine or Proton processes);
  - a process of his matches GAME below (the same, plus Minecraft and a Horizon game he started himself);
  - a process outside horizon-ci.slice keeps the GPU busy (nvidia-smi: SM use >= GPU_BUSY % twice in a row);
  - ~/.actions-runners/horizon/pause exists (pause by hand: `touch` it; `rm` it to let the watcher decide).

Paused, the runner takes no new job: while it is idle its service is stopped, so GitHub sends the jobs to the
Mac mini. A job already running is throttled, not stopped: horizon-ci.slice (the job, and the parity workers other machines
send here) drops to CPU weight 1, I/O weight 1
and a quota of THROTTLE_CPUS threads, and the service stops once the job ends. Freezing it (SIGSTOP or the cgroup
freezer) would stop the runner's heartbeat, so GitHub would drop the job as lost, and the harnesses' wall-clock
timeouts would fail runs that were only waiting. The runner comes back RESUME_AFTER seconds after the last sign
of a game. Every change is written to ~/.actions-runners/horizon/pause.log.
"""
import json
import os
import re
import subprocess
import time
from pathlib import Path

DIR = Path.home() / ".actions-runners" / "horizon"
SERVICE, SLICE = "horizon-runner.service", "horizon-ci.slice"
MANUAL, STATE, LOG = DIR / "pause", DIR / "paused", DIR / "pause.log"
GAME = re.compile(r"(SteamLaunch|gamescope|wine64-preloader|wineserver|wine-preloader|\.exe(\s|$)|proton\b"
                  r"|net\.minecraft\.client\.main\.Main|minecraft-launcher|prismlauncher)", re.I)
GPU_BUSY = int(os.environ.get("GPU_BUSY", 30))
INTERVAL = int(os.environ.get("INTERVAL", 15))
RESUME_AFTER = int(os.environ.get("RESUME_AFTER", 300))
THROTTLE_CPUS = int(os.environ.get("THROTTLE_CPUS", 2))


def log(text):
    with LOG.open("a") as handle:
        handle.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {text}\n")


def systemctl(*arguments):
    return subprocess.run(["systemctl", "--user", *arguments], capture_output=True, text=True)


def ci_process(pid):
    """A process of the runner's slice (its jobs, the parity workers other machines start here)."""
    try:
        return SLICE in Path(f"/proc/{pid}/cgroup").read_text()
    except OSError:
        return True  # gone


def fleet_load_gaming():
    status = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")) / "fleet-load" / "status.json"
    try:
        if time.time() - status.stat().st_mtime > 120:
            return None
        return "fleet-load: playing a game" if json.loads(status.read_text())["human"]["gaming"] else None
    except (OSError, ValueError, KeyError, TypeError):
        return None


def game_process():
    uid = os.getuid()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != uid:
                continue
            command = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        except OSError:
            continue
        # A Horizon game counts when he started it (its parent is his launcher), not when a harness or agent did.
        if GAME.search(command) or (" --offscreen" not in command and "/horizon-launcher/" in command
                                    and "horizon-client" in command.split(" ")[0]):
            if not ci_process(int(entry.name)):
                return f"process {entry.name}: {command[:120]}"
    return None


def gpu_users():
    """Processes outside the CI slice using the GPU's shader cores, as {pid: SM %}."""
    try:
        output = subprocess.run(["nvidia-smi", "pmon", "-c", "1", "-s", "u"], capture_output=True, text=True,
                                timeout=20).stdout
    except (OSError, subprocess.TimeoutExpired):
        return {}
    users = {}
    for line in output.splitlines():
        fields = line.split()
        if line.startswith("#") or len(fields) < 4 or not fields[1].isdigit() or not fields[3].isdigit():
            continue
        if not ci_process(int(fields[1])):
            users[int(fields[1])] = int(fields[3])
    return users


def runner_busy():
    """The runner is in a job: a Runner.Worker runs in its service."""
    group = systemctl("show", "-p", "ControlGroup", "--value", SERVICE).stdout.strip()
    try:
        pids = Path(f"/sys/fs/cgroup{group}/cgroup.procs").read_text().split() if group else []
    except OSError:
        return False
    for pid in pids:
        try:
            if Path(f"/proc/{pid}/comm").read_text().strip() == "Runner.Worker":
                return True
        except OSError:
            pass
    return False


def throttle(on):
    if on:
        systemctl("set-property", "--runtime", SLICE, "CPUWeight=1", "IOWeight=1", f"CPUQuota={THROTTLE_CPUS * 100}%")
    else:
        systemctl("set-property", "--runtime", SLICE, "CPUWeight=20", "IOWeight=20", "CPUQuota=")


def main():
    hot = {}  # GPU users seen busy at the last sample
    last_game = 0.0
    paused = STATE.exists()
    while True:
        reason = None
        if MANUAL.exists():
            reason = "paused by hand (" + str(MANUAL) + ")"
        reason = reason or fleet_load_gaming() or game_process()
        if not reason:
            busy = {pid for pid, sm in gpu_users().items() if sm >= GPU_BUSY}
            twice = busy & set(hot)
            hot = dict.fromkeys(busy)
            if twice:
                pid = min(twice)
                try:
                    name = Path(f"/proc/{pid}/comm").read_text().strip()
                except OSError:
                    name = "?"
                reason = f"GPU busy: process {pid} ({name})"
        now = time.monotonic()
        if reason:
            last_game = now
        if reason and not paused:
            paused = True
            STATE.write_text(reason + "\n")
            throttle(True)  # also what other machines' parity workers run here meanwhile
            if runner_busy():
                log(f"pause: {reason}; a job is running: throttled to {THROTTLE_CPUS} threads, stops after it")
            else:
                systemctl("stop", SERVICE)
                log(f"pause: {reason}; runner stopped")
        elif paused and reason is None and now - last_game >= RESUME_AFTER:
            paused = False
            STATE.unlink(missing_ok=True)
            throttle(False)
            systemctl("start", SERVICE)
            log(f"resume: no game for {RESUME_AFTER} s; runner started")
        elif paused and systemctl("is-active", "--quiet", SERVICE).returncode == 0 and not runner_busy():
            systemctl("stop", SERVICE)
            log("the job ended: runner stopped until the game ends")
        time.sleep(INTERVAL)


if __name__ == "__main__":
    main()
