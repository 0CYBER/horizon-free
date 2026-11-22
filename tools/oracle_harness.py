"""Shared plumbing for single-oracle differential harnesses.

A harness supplies a Java oracle (compiled against the pinned client jar and its
libraries), a Rust example probe, and a text input; both programs write one
result line per input line and the outputs must be byte-identical."""
import json
import os
from pathlib import Path
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
    from .integration_env import run_game
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking
    from integration_env import run_game


def run_logged(command, build, name, timeout=900, **kwargs):
    path = build / (name + '.log')
    with path.open('w') as log:
        runner = run_game if Path(command[0]).name == 'horizon-client' else subprocess.run
        try:
            runner([str(c) for c in command], cwd=lab.ROOT, check=True, timeout=timeout,
                   stdout=log, stderr=subprocess.STDOUT, **kwargs)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            if compile_only(command):
                try:
                    log.flush()
                    print_log_tail(path)
                except Exception:  # the report is best effort: the caller gets the command's own error
                    pass
            raise


def compile_only(command):
    """A cargo command that only compiles (`cargo build`, `cargo test --no-run`): its log holds Cargo's and the
    compiler's messages about Horizon's own code. Anything that runs a program (a test run, a Java oracle, the game)
    can print vanilla game data, which never goes into the public artifact, so its log is never shown."""
    words = [str(word) for word in command]
    if not words or Path(words[0]).name not in ('cargo', 'cargo.exe') or len(words) < 2:
        return False
    words = words[:words.index('--')] if '--' in words else words
    return words[1] == 'build' or (words[1] == 'test' and '--no-run' in words)


LOG_TAIL_BYTES = 256 << 10  # of the log read
LOG_LINE_CHARS = 400  # per line shown


def print_log_tail(path, lines=40):
    """The end of a failed compile's log, on stderr: only the harness's own output reaches the parity run's
    artifact, so a failed build otherwise shows just its exit code. Cargo's JSON messages print as the text it
    would have shown (an error's `rendered` field), other JSON lines not at all. The checkout and home folder
    show as <checkout> and ~."""
    with path.open('rb') as file:
        start = max(0, file.seek(0, os.SEEK_END) - LOG_TAIL_BYTES)
        file.seek(start)
        data = file.read(LOG_TAIL_BYTES)  # a timed-out build's children may still be writing
    if start:
        data = data.partition(b'\n')[2]  # the cut line: half a JSON record would print as plain text
    shown = []
    for line in data.decode('utf-8', errors='replace').splitlines():
        if line.startswith('{'):
            try:
                message = json.loads(line)
            except ValueError:
                shown.append(line)
                continue
            inner = message.get('message') if isinstance(message, dict) else None
            if isinstance(inner, dict) and inner.get('level') in ('error', 'error: internal compiler error') \
                    and isinstance(inner.get('rendered'), str):
                shown.extend(inner['rendered'].rstrip().splitlines())
            continue
        shown.append(line)
    tail = [redact(line)[:LOG_LINE_CHARS] for line in shown[-lines:]]
    if tail:
        print(f'--- last {len(tail)} lines of {path.name} ---', file=sys.stderr)
        print('\n'.join(tail), file=sys.stderr, flush=True)


def redact(line):
    for folder, name in ((str(lab.ROOT), '<checkout>'), (str(Path.home()), '~')):
        if len(folder) > 1:
            line = line.replace(folder, name)
    return line


def compile_oracle(build, oracle_name, jars, libraries, java_major):
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity' / (oracle_name + '.java')
    classes = build / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    run_logged(['javac', '--release', java_major, '-proc:none', '-sourcepath',
                lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
               build, 'compile-java-' + oracle_name)
    return classes, dependencies, oracle


def build_probe(build, example):
    """Build the horizon-client probe binary; returns the command that runs probe `example`."""
    run_logged(['cargo', 'build', '--locked', '-p', 'horizon-client-lib', '--example', 'client_probe'],
               build, 'compile-rust-' + example, env=dict(os.environ, CARGO_BUILD_JOBS='3'))
    import json
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    return [Path(metadata['target_directory']) / 'debug/examples/client_probe', example]


def java_run(build, classes, dependencies, main_class, args, name, jvm=()):
    run_logged(['java', '-Xmx2g', '-Djava.awt.headless=true', *jvm, '-cp', str(classes) + os.pathsep + dependencies,
                'dev.horizonlab.parity.' + main_class, *args], build, name)


def compare(client_out, rust_out, expected_lines):
    matched = base.compare(client_out, rust_out)
    if matched != expected_lines:
        raise AssertionError(f'Incomplete output: {matched} of {expected_lines} lines')
    return matched


def differential(name, oracle, probe, make_input, scope, sources=(), expected_lines=None):
    """Compile the oracle and probe, run both on one input and require identical output."""
    import platform
    lock, jars, libraries = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / name
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle_path = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity' / (oracle + '.java')
    all_sources = [Path(__file__).resolve(), lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
                   lab.ROOT / 'tools/lab.py', oracle_path, lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
                   lab.ROOT / 'crates/horizon-client/Cargo.toml', lab.ROOT / 'crates/horizon-client/src/lib.rs',
                   lab.ROOT / f'crates/horizon-client/examples/probe/{probe}.rs', *map(Path, sources)]
    tested = base.source_hashes(all_sources)
    input_path, units = make_input(build)
    input_hash = lab.digest(input_path, 'sha256') if input_path.is_file() else None
    classes, dependencies, _ = compile_oracle(build, oracle, jars, libraries, lock['minecraft']['java_major'])
    command = build_probe(build, probe)
    run_logged([*command, input_path, build / 'rust.txt'], build, 'rust')
    java_run(build, classes, dependencies, oracle, [input_path, build / 'client.txt'], 'client')
    try:
        matched = base.compare(build / 'client.txt', build / 'rust.txt')
    except AssertionError as error:
        raise AssertionError(str(error) + explain(build, str(error))) from None
    if expected_lines is not None and matched != expected_lines:
        raise AssertionError(f'Incomplete output: {matched} of {expected_lines} lines')
    base.ensure_sources_unchanged(all_sources, tested)
    if input_hash is not None and input_hash != lab.digest(input_path, 'sha256'):
        raise AssertionError('Input fixtures changed')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'],
        platform=platform.platform(), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0], units=units,
        sources_sha256=tested, input_sha256=input_hash, client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        matched_output_lines=matched, output_sha256=lab.digest(build / 'rust.txt', 'sha256'), scope=scope))
    print(f'{units:,} {name} cases match the pinned client ({matched:,} output lines): {report}', flush=True)
    return matched


def explain(build, message):
    """Summarise differing sections ("== name" blocks) of multi-case outputs."""
    def sections(path):
        out, name = {}, None
        for line in path.read_text(encoding='utf-8', errors='replace').splitlines():
            if line.startswith('== '):
                name = line[3:]
                out[name] = []
            elif name is not None:
                out[name].append(line)
        return out
    left, right = sections(build / 'client.txt'), sections(build / 'rust.txt')
    report, count = '', 0
    for name in sorted(set(left) | set(right)):
        if left.get(name) != right.get(name):
            count += 1
            if count <= 25:
                case = build / 'cases' / name
                text = case.read_text(encoding='utf-8', errors='replace') if case.is_file() else ''
                a, b = left.get(name, []), right.get(name, [])
                diffs = [(x, y) for x, y in zip(a, b) if x != y][:4]
                if len(a) != len(b):
                    diffs.append((f'<{len(a)} lines>', f'<{len(b)} lines>'))
                report += f"\n  case {name}: {text[:300]!r}\n    differences (vanilla, rust): {diffs}"
    return f'\n{count} differing cases.' + report
