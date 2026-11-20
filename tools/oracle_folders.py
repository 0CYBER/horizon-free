"""A folder of its own for each vanilla oracle call a harness makes.

tools/parity_shim.py replays an oracle call only while the files under the paths the call names are the ones it
recorded: its inputs with the same content and no file added beside them, and its outputs exactly as the call left
them (size and mtime). A harness that hands an oracle its whole work folder breaks that on every run: its later steps
add files there (the Rust side's output, logs, report.json, the next call's output), and a log it opens there before
the call is one of the call's outputs, changed again.

folder() gives a call a folder that holds only the inputs it is given, copied in, and what the call writes there. The
call's log, the Rust side's files and the report stay outside it. A replay writes no files, so the folder is kept
between runs; but a call that runs Java must never read a file an earlier, different call left there (an output it
does not write again, an input no longer given). So the folder is named after a digest of what defines the call: the
inputs' names and contents, the oracle's compiled classes, its other arguments, the JVM options the environment can
add (which can change what the oracle writes: VanillaCollisionOracle's horizon.prepare.reuse) and the Java runtime.
The same call finds the same folder, under the same path on every machine; any other call starts in an empty one. The
call's folders under other digests are removed.
"""

import hashlib
import os
from pathlib import Path
import re
import shutil

# The variables the JVM and javac read options from (tools/parity_shim.py's UNKEYED, which a test keeps equal): not
# imported, so that a change to the shim doesn't count as a change to every harness using this module.
UNKEYED = ("JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "JDK_JAVAC_OPTIONS", "CLASSPATH")


def folder(path, *inputs, call=()):
    """The folder for one oracle call, `path` with the call's digest appended, holding a copy of each of `inputs`
    under its own name. `call`: what else defines the call, as strings or paths: a folder (the compiled oracle)
    counts by the names and bytes of its files, a file (a jar, its content pinned by research-lock.json) by name and size."""
    path = Path(path)
    digest = hashlib.sha256()

    def add(*parts):
        for part in parts:
            data = part if isinstance(part, bytes) else str(part).encode()
            digest.update(len(data).to_bytes(8, "little") + data)

    for source in inputs:
        add("input", Path(source).name, hashlib.sha256(Path(source).read_bytes()).digest())
    for part in call:
        if isinstance(part, Path) and part.is_dir():
            for file in sorted(item for item in part.rglob("*") if item.is_file()):
                add("folder", file.relative_to(part).as_posix(), file.read_bytes())
        elif isinstance(part, Path) and part.is_file():
            add("file", part.name, part.stat().st_size)
        else:
            add("argument", part)
    for variable in UNKEYED:
        add("variable", variable, os.environ.get(variable, ""))
    add("runtime", runtime())
    own = path.with_name(f"{path.name}-{digest.hexdigest()[:16]}")
    pattern = re.compile(re.escape(path.name) + r"(-[0-9a-f]{16})?")
    if path.parent.is_dir():
        for other in path.parent.iterdir():
            if other != own and pattern.fullmatch(other.name) and other.is_dir() and not other.is_symlink():
                shutil.rmtree(other)
    own.mkdir(parents=True, exist_ok=True)
    for source in inputs:
        shutil.copyfile(source, own / Path(source).name)
    return own


def runtime():
    """The Java runtime a call starts, cheaply: its JDK's `release` file (version, vendor, build), else the launcher's
    path. Under tools/parity.py, `java` on PATH is the shim: the real one is on PARITY_SHIM_REAL_PATH."""
    found = shutil.which("java", path=os.environ.get("PARITY_SHIM_REAL_PATH") or None)
    if not found:
        return "none"
    launcher = Path(os.path.realpath(found))
    try:
        return (launcher.parent.parent / "release").read_bytes()
    except OSError:
        return str(launcher)


def copy(path, target, *names):
    """Copy the files `names` a call wrote in its folder `path` to `target`, for the harness's later steps to read
    there: what they write beside them then stays out of the call's folder."""
    for name in names:
        shutil.copyfile(Path(path) / name, Path(target) / name)
