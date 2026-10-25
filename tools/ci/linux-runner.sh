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
# the work folder, and copies of this script and runner-pause.py taken at install time (a pull request's checkout
# never runs as the service). The service runs nice, in horizon-ci.slice (low CPU and I/O weight, memory capped),
# on its own virtual screen: vscreen (a headless sway) where it is installed, else Xvfb from nixpkgs with Mesa's
# Vulkan drivers. runner-pause.py pauses it while a game runs (horizon-runner-pause.service).
# HORIZON_SHARED_SOURCE: the imported vanilla data (default ~/horizon-local/shared-local, else
# ~/code/horizon/.local); RUNNER_JOBS: cargo jobs and harnesses at once (default 2/3 of the threads).
set -euo pipefail
repo=ludovic111/horizon
dir=$HOME/.actions-runners/horizon
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
version=2.338.0
sha=af4b794c1bc41d73d40535e3fe092a39f9679cd8d965954c2aca25a05ca41d32
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# Nix packages the jobs need, kept alive by GC roots in $dir/nix.
nix_out() { nix-build '<nixpkgs>' -A "$1" -o "$dir/nix/$2" > /dev/null && readlink -f "$dir/nix/$2"; }

# The runner is a .NET program: on NixOS its native libraries (libstdc++, ICU, OpenSSL) come from nix-ld.
export LD_LIBRARY_PATH=/run/current-system/sw/share/nix-ld/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
# NixOS has no /bin/bash: the runner's scripts (again after each of its self-updates) get a portable one.
fix_shebangs() { sed -i '1s|^#!/bin/bash|#!/usr/bin/env bash|' "$dir"/*.sh "$dir"/bin/*.sh; }

display_env() {
  # The jobs' screen. Never the maintainer's: the user manager hands every service his DISPLAY and WAYLAND_DISPLAY.
  if command -v vscreen > /dev/null; then
    VSCREEN=ci-horizon vscreen env
  else
    echo "unset WAYLAND_DISPLAY; export DISPLAY=:$(cat "$dir/display")"
  fi
}

install() {
  local name=${RUNNER_NAME:-$(hostname)-horizon} jobs=${RUNNER_JOBS:-$(( $(nproc) * 2 / 3 ))}
  local source=${HORIZON_SHARED_SOURCE:-}
  if [ -z "$source" ]; then
    for candidate in "$HOME/horizon-local/shared-local" "$HOME/code/horizon/.local"; do
      [ -d "$candidate/minecraft" ] && { source=$candidate; break; }
    done
  fi
  [ -n "$source" ] && [ -d "$source/minecraft" ] || { echo "no imported Minecraft (HORIZON_SHARED_SOURCE)"; exit 1; }
  mkdir -p "$dir/nix" "$units"
  cp "$here/linux-runner.sh" "$here/runner-pause.py" "$dir/"

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
  local alsa udev java sccache
  alsa=$(nix_out alsa-lib.dev alsa-dev)
  udev=$(nix_out systemd.dev systemd-dev)
  java=$(nix_out "jdk$(sed -n 's/.*"java_major": *\([0-9]*\).*/\1/p' "$here/../../research-lock.json" | head -1)_headless" jdk)/lib/openjdk
  sccache=$(nix_out sccache sccache)
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
Description=GitHub Actions runner for $repo (parity), paused while a game runs
After=network-online.target
[Service]
Slice=horizon-ci.slice
WorkingDirectory=$dir
ExecStart=/usr/bin/env bash $dir/linux-runner.sh run
Nice=10
# runsvc.sh hands SIGTERM to the runner, which stops after cancelling its job; then the rest (Xvfb) goes.
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=5min
Restart=on-failure
RestartSec=30
[Install]
WantedBy=default.target
EOF
  cat > "$units/horizon-runner-pause.service" <<EOF
[Unit]
Description=Pauses the Horizon CI runner while a game runs
[Service]
ExecStart=$(command -v python3) $dir/runner-pause.py
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
  systemctl --user enable horizon-runner.service horizon-runner-pause.service > /dev/null
  systemctl --user restart horizon-runner-pause.service
  [ -e "$dir/paused" ] || systemctl --user start horizon-runner.service
  status
}

run() {
  cd "$dir"
  fix_shebangs
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
