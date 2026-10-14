#!/usr/bin/env bash
# The tools the vanilla harnesses need, on the self-hosted Mac mini runner (label horizon-game): its service
# starts with a bare PATH. Python 3.13 and the JDK pinned by research-lock.json come from the machine, never
# from a download. Writes them to $GITHUB_PATH and $GITHUB_ENV for the next steps.
set -euo pipefail
java_major=$(sed -n 's/.*"java_major": *\([0-9]*\).*/\1/p' research-lock.json | head -1)
bin="$RUNNER_TEMP/horizon-bin"
mkdir -p "$bin"
if [ "$(uname)" = Darwin ]; then
  ln -sfn /opt/homebrew/bin/python3.13 "$bin/python3"
  ln -sfn /opt/homebrew/bin/python3.13 "$bin/python"
  java_home=$(/usr/libexec/java_home -v "$java_major")
  { echo "$bin"; echo "$java_home/bin"; echo /opt/homebrew/bin; } >> "$GITHUB_PATH"
  echo "JAVA_HOME=$java_home" >> "$GITHUB_ENV"
  # Builds go through the machine's sccache (shared with ~/fleet/rbuild), so the build folders runner-gc.sh
  # removes come back from the cache instead of compiling cold.
  if [ -x /opt/homebrew/bin/sccache ]; then
    { echo "RUSTC_WRAPPER=/opt/homebrew/bin/sccache"; echo "CARGO_INCREMENTAL=0"; } >> "$GITHUB_ENV"
  fi
fi
echo "HORIZON_NO_AUDIO=1" >> "$GITHUB_ENV"
