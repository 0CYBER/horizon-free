"""Linux verify network guard: binary-parallel execution with isolated diagnosis on violations."""
import os
import re
import tempfile
from pathlib import Path


class ListingFailed(Exception):
    """The guard cannot enumerate the selected tests safely."""


def invocations(binaries, env, test_args, list_tests):
    """List selections without preloading; remove ignored tests before isolating each invocation."""
    # Listing uses the caller's filters; execution must use only the scheduled name.
    # Preserve values belonging to options rather than treating them as filters.
    options, takes_value = [], False
    for arg in test_args:
        if takes_value:
            options.append(arg)
            takes_value = False
        elif arg == "--":
            raise ListingFailed("network guard does not accept an additional -- in test arguments")
        elif arg == "--exact":
            continue  # Each invocation supplies this once, after selection.
        elif arg.startswith("-"):
            options.append(arg)
            takes_value = arg in {
                "-Z", "--skip", "--test-threads", "--format", "--color", "--logfile", "--shuffle-seed",
            }
    for binary in binaries:
        if not binary.get("libtest", True):
            yield binary, test_args
            continue
        local_env = {**os.environ, **env, **binary["env"]}
        names = list_tests(binary, local_env, test_args)
        ignored = list_tests(binary, local_env, [*test_args, "--ignored"])
        if names is None or ignored is None:
            raise ListingFailed(f"{binary['label']}: cannot list tests for network guard")
        for name in names:
            if name not in ignored:
                yield {**binary, "label": f"{binary['label']}: {name}", "isolated_test": name}, [
                    *options, "--exact", name]


def run(invocation, env, timed, shim, run_one):
    binary, args = invocation
    with tempfile.TemporaryDirectory(prefix="verify-network-") as scratch:
        log = Path(scratch, "violations")
        log.touch()
        preload = os.pathsep.join(filter(None, [str(shim), os.environ.get("LD_PRELOAD")]))
        result = run_one(binary, {**env, "LD_PRELOAD": preload, "HORIZON_NETWORK_GUARD_LOG": str(log)},
                         args, timed, listing_env=env)
        records = log.read_text().splitlines()
        markers = {line for line in records if re.fullmatch(r"loaded [0-9]+", line)}
        operations = sorted(set(records) - markers)
        result["network_operations"] = operations
        if f"loaded {result.get('pid')}" not in markers or operations:
            detail = ", ".join(operations) if operations else "guard library did not load"
            result["status"] = f"network guard: {detail}"
            result["output"].write(f"\n::error::network guard: {binary['label']}: {detail}; "
                                   'mark it #[ignore = "daily"] and add its daily row, reason network '
                                   '(tools/ci/move_to_daily.py)\n')
        return result


def run_binary(binary, env, test_args, timed, shim, run_one, list_tests):
    """Keep green binaries intact; isolate only a binary which recorded networking."""
    result = run((binary, test_args), env, timed, shim, run_one)
    if not result["network_operations"] or not binary.get("libtest", True):
        return result
    result["network_tests"] = []
    try:
        for invocation in invocations([binary], env, test_args, list_tests):
            isolated = run(invocation, env, False, shim, run_one)
            if isolated["network_operations"]:
                name = invocation[0]["isolated_test"]
                result["network_tests"].append(name)
                isolated["output"].seek(0)
                result["output"].write(isolated["output"].read())
            isolated["output"].close()
    except ListingFailed as error:
        result["output"].write(f"\n::error::{error}\n")
    if not result["network_tests"]:
        result["output"].write(f"\n::error::network guard: {binary['label']}: binary recorded networking "
                               "but no isolated culprit was found; this may depend on shared test state "
                               "or a late child. The binary still fails.\n")
    return result
