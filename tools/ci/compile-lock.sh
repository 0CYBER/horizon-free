#!/usr/bin/env bash
# compile-lock.sh <role> -- <command>...: the command in a turn on the self-hosted Mac (tools/compile_lock.py), for
# game-build.yml's Mac compiles. The Python process holds the turn and runs the command as its child; the command
# never inherits the lock. The build job compiles before runner-env.sh puts Homebrew's Python on PATH, so this takes
# whichever python3 runs the module (/usr/bin/python3 then); with none, the command runs without a turn.
role=$1
shift
[ "${1:-}" = -- ] && shift
script="${0%/*}/../compile_lock.py"
for python in python3.13 python3; do
  if command -v "$python" > /dev/null 2>&1 && "$python" "$script" --probe > /dev/null 2>&1; then
    exec "$python" "$script" "$role" -- "$@"
  fi
done
echo "  compile lock: no Python runs tools/compile_lock.py here; compiling without it"
exec "$@"
