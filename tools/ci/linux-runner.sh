#!/usr/bin/env bash
# A Horizon CI runner (labels self-hosted, Linux, X64, horizon-game and the host's name) on a Linux machine of the
# maintainer's (NixOS: ludodesktop, ludothinkpad), as a systemd user service of the account that holds his
# Minecraft files, like the Mac mini's runners (docs/release-flow.md). Run it on that machine, as that user:
#
#   bash tools/ci/linux-runner.sh install    # register (gh login), install, start; again to update the scripts
#   bash tools/ci/linux-runner.sh status     # the services, the runner's state, the pause watcher's last lines
#   bash tools/ci/linux-runner.sh uninstall  # stop and unregister
#
# Everything lives in ~/.actions-runners/horizon (never in ~/code): the runner, its own rustup and cargo homes,
# the work folder, and copies of this script and runner_pause.py taken at install time (a pull request's checkout
# never runs as the service). horizon-runner-pause.service (runner_pause.py) starts the runner only while nobody
# uses the machine (screen locked or untouched for 15 min, no game), and stops it otherwise. The runner runs nice,
# in horizon-ci.slice (low CPU and I/O weight, memory capped), on its own virtual screen: vscreen (a headless sway)
# where it is installed, else Xvfb from nixpkgs with Mesa's Vulkan drivers.
# HORIZON_SHARED_SOURCE: the imported vanilla data (default ~/horizon-local/shared-local, else
# ~/code/horizon/.local); RUNNER_JOBS: cargo jobs and harnesses at once (default a third of the threads, at most 8:
# ludodesktop froze under ~7 cold cargo builds on 2026-10-14, and it is the maintainer's own machine).
set -euo pipefail
repo=ludovic111/horizon
dir=$HOME/.actions-runners/horizon
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
version=2.338.0
sha=af4b794c1bc41d73d40535e3fe092a39f9679cd8d965954c2aca25a05ca41d32
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# Nix packages the jobs need, kept alive by GC roots in $dir/nix. nix-build prints the store path; the root's name
# gets the output's suffix (alsa-dev-dev for alsa-lib.dev), so it is not read back from the link.
nix_out() { nix-build '<nixpkgs>' -A "$1" -o "$dir/nix/$2" | tail -1; }

# The runner is a .NET program: on NixOS its native libraries (libstdc++, ICU, OpenSSL) come from nix-ld.
export LD_LIBRARY_PATH=/run/current-system/sw/share/nix-ld/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
# NixOS has no /bin/bash: the runner's scripts (again after each of its self-updates) get a portable one.
fix_shebangs() { sed -i '1s|^#!/bin/bash|#!/usr/bin/env bash|' "$dir"/*.sh "$dir"/bin/*.sh; }

# Other machines' parity runs can send harnesses here (tools/parity_hosts.json "desktop": parity.py --worker over
# ssh, in horizon-ci.slice). Their vanilla data is the runner checkout's: parity-host/shared-local links to its
# .local links, so it follows the preparation of the runner's last job.
parity_host_links() {
  local checkout=$dir/_work/horizon/horizon/.local shared=$dir/parity-host/shared-local entry
  [ -d "$checkout" ] || return 0
  mkdir -p "$shared"
  for entry in "$checkout"/*; do
    if [ -L "$entry" ]; then ln -sfn "$entry" "$shared/$(basename "$entry")"; fi
  done
}

display_env() {
  # The jobs' screen. Never the maintainer's: the user manager hands every service his DISPLAY and WAYLAND_DISPLAY.
  if command -v vscreen > /dev/null; then
    VSCREEN=ci-horizon vscreen env
  else
    echo "unset WAYLAND_DISPLAY; export DISPLAY=:$(cat "$dir/display")"
  fi
}

install() {
  local name=${RUNNER_NAME:-$(hostname)-horizon} threads jobs
  threads=$(nproc)
  jobs=${RUNNER_JOBS:-$(( threads / 3 > 8 ? 8 : threads / 3 > 1 ? threads / 3 : 1 ))}
  local source=${HORIZON_SHARED_SOURCE:-}
  if [ -z "$source" ]; then
    for candidate in "$HOME/horizon-local/shared-local" "$HOME/code/horizon/.local"; do
      [ -d "$candidate/minecraft" ] && { source=$candidate; break; }
    done
  fi
  [ -n "$source" ] && [ -d "$source/minecraft" ] || { echo "no imported Minecraft (HORIZON_SHARED_SOURCE)"; exit 1; }
  mkdir -p "$dir/nix" "$units"
  cp "$here/linux-runner.sh" "$here/runner_pause.py" "$dir/"

  if [ ! -f "$dir/.runner" ]; then
    local cache=$HOME/.cache/gha-runner/actions-runner-linux-x64-$version.tar.gz
    if ! echo "$sha  $cache" | sha256sum -c --quiet - > /dev/null 2>&1; then
      mkdir -p "$(dirname "$cache")"
      curl -fsSL -o "$cache" "https://github.com/actions/runner/releases/download/v$version/actions-runner-linux-x64-$version.tar.gz"
      echo "$sha  $cache" | sha256sum -c --quiet -
    fi
    tar xzf "$cache" -C "$dir"
    fix_shebangs
    gh api -X POST "repos/$repo/actions/runners/registration-token" --jq .token |
      (cd "$dir" && bash ./config.sh --unattended --replace --url "https://github.com/$repo" --token "$(cat)" \
        --name "$name" --labels "${RUNNER_LABELS:-horizon-game,$(hostname)}" --work _work > /dev/null)
  fi

  # Libraries for building (pkg-config) and running (dlopen) the game, the JDK pinned by research-lock.json, sccache.
  local alsa udev java
  alsa=$(nix_out alsa-lib.dev alsa-dev)
  udev=$(nix_out systemd.dev systemd-dev)
  java=$(nix_out "jdk$(sed -n 's/.*"java_major": *\([0-9]*\).*/\1/p' "$here/../../research-lock.json" | head -1)_headless" jdk)/lib/openjdk
  nix_out sccache sccache > /dev/null  # on the PATH below: runner-env.sh hands it to cargo
  local libs=/run/opengl-driver/lib:/run/current-system/sw/share/nix-ld/lib
  if ! command -v vscreen > /dev/null; then
    local mesa x
    mesa=$(nix_out mesa mesa)
    x=$(nix_out xvfb-run xvfb-run)
    for lib in libx11 libxcursor libxrandr libxi libxext libxinerama libxrender libxxf86vm libxcb libxkbcommon \
        vulkan-loader libGL; do
      libs=$libs:$(nix_out "$lib" "$lib")/lib
    done
    libs=$mesa/lib:$libs
    echo "VK_DRIVER_FILES=$mesa/share/vulkan/icd.d/intel_icd.x86_64.json:$mesa/share/vulkan/icd.d/lvp_icd.x86_64.json" \
      > "$dir/.env.display"
    echo "$x/bin" > "$dir/.path.display"
  fi
  cat > "$dir/.env" <<EOF
LANG=C.UTF-8
RUSTUP_HOME=$dir/rustup
CARGO_HOME=$dir/cargo
CARGO_BUILD_JOBS=$jobs
PARITY_MACHINE_JOBS=$jobs
PARITY_LOCAL_JOBS=$jobs
JAVA_HOME=$java
HORIZON_SHARED_SOURCE=$source
HORIZON_NO_AUDIO=1
HORIZON_NO_HEARTBEAT=1
HORIZON_TELEMETRY_URL=off
HORIZON_LAUNCHER_NO_SELF_UPDATE=1
PKG_CONFIG_PATH=$alsa/lib/pkgconfig:$udev/lib/pkgconfig
C_INCLUDE_PATH=$udev/include
LD_LIBRARY_PATH=$libs
NIX_LD=/run/current-system/sw/share/nix-ld/lib/ld.so
NIX_LD_LIBRARY_PATH=/run/current-system/sw/share/nix-ld/lib
EOF
  if [ -f "$dir/.env.display" ]; then cat "$dir/.env.display" >> "$dir/.env"; else echo "VSCREEN=ci-horizon" >> "$dir/.env"; fi
  echo "$dir/cargo/bin:$(cat "$dir/.path.display" 2>/dev/null || true)${dir}/nix/sccache/bin:$java/bin:$HOME/.local/bin:/run/wrappers/bin:/etc/profiles/per-user/$USER/bin:/run/current-system/sw/bin" \
    | sed 's/::/:/g' > "$dir/.path"

  cat > "$units/horizon-ci.slice" <<EOF
[Unit]
Description=Horizon CI: the runner's jobs and parity workers sent from other machines
[Slice]
CPUWeight=20
IOWeight=20
MemoryHigh=$(( $(awk '/MemTotal/ {print int($2 / 1048576)}' /proc/meminfo) / 2 ))G
MemoryMax=$(( $(awk '/MemTotal/ {print int($2 / 1048576)}' /proc/meminfo) * 2 / 3 ))G
EOF
  cat > "$units/horizon-runner.service" <<EOF
[Unit]
Description=GitHub Actions runner for $repo (parity), started and stopped by horizon-runner-pause
After=network-online.target
[Service]
Slice=horizon-ci.slice
WorkingDirectory=$dir
ExecStart=/usr/bin/env bash $dir/linux-runner.sh run
Nice=19
# runsvc.sh hands SIGTERM to the runner, which stops after cancelling its job; then the rest (Xvfb) goes.
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=5min
Restart=on-failure
RestartSec=30
EOF
  cat > "$units/horizon-runner-pause.service" <<EOF
[Unit]
Description=Runs the Horizon CI runner only while nobody uses the machine or plays on it
[Service]
ExecStart=$(command -v python3) $dir/runner_pause.py
Restart=always
RestartSec=10
[Install]
WantedBy=default.target
EOF
  # User services run without a login session (after a reboot): lingering, once.
  if [ "$(loginctl show-user "$USER" -p Linger --value 2>/dev/null)" != yes ]; then
    sudo -n loginctl enable-linger "$USER" || echo "Run once: sudo loginctl enable-linger $USER"
  fi
  systemctl --user daemon-reload
  # Only the watcher starts the runner (never the boot itself): it decides whether anyone is at the screen.
  systemctl --user enable horizon-runner-pause.service > /dev/null
  # The runner reads .env when it starts: stop it, unless it is in a job (that would cancel the job); the watcher
  # starts it again when it may run.
  if pgrep -f "$dir/bin/Runner.Worker" > /dev/null; then echo "In a job: stop horizon-runner.service after it"
  else systemctl --user stop horizon-runner.service; rm -f "$dir/paused"; fi
  systemctl --user restart horizon-runner-pause.service
  status
}

run() {
  cd "$dir"
  fix_shebangs
  parity_host_links
  if command -v vscreen > /dev/null; then
    eval "$(display_env)"
  else
    # Xvfb on the first free display from :90, for this service's lifetime.
    local n=90
    while [ -e "/tmp/.X$n-lock" ]; do n=$((n + 1)); done
    echo "$n" > "$dir/display"
    Xvfb ":$n" -screen 0 1280x720x24 -nolisten tcp &
    eval "$(display_env)"
  fi
  exec bash ./bin/runsvc.sh
}

status() {
  systemctl --user --no-pager list-units 'horizon-runner*' 'vscreen-ci-horizon*'
  [ -e "$dir/paused" ] && echo "paused: $(cat "$dir/paused")"
  tail -5 "$dir/pause.log" 2>/dev/null || true
}

uninstall() {
  systemctl --user disable --now horizon-runner-pause.service horizon-runner.service 2>/dev/null || true
  gh api -X POST "repos/$repo/actions/runners/remove-token" --jq .token |
    (cd "$dir" && bash ./config.sh remove --token "$(cat)")
  rm -f "$units/horizon-runner.service" "$units/horizon-runner-pause.service" "$units/horizon-ci.slice"
  systemctl --user daemon-reload
}

case ${1:-} in
  install|run|status|uninstall) "$1" ;;
  *) sed -n '2,10p' "$0"; exit 2 ;;
esac
