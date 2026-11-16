#!/usr/bin/env python3
"""Compare bed POI classification with the actual vanilla 26.3 block objects.

Checks both runtime and saved POI classifiers. Outputs are private dev data, not game resources.
"""
import os
from pathlib import Path

try:
    from . import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking


def main():
    build = Path(os.environ.get("HORIZON_BED_POI_PARITY_DIR", lab.LOCAL / "bed-poi-parity"))
    build.mkdir(parents=True, exist_ok=True)
    lock, jars, _ = base.verify_artifacts()
    classes, dependencies, _ = oracle_harness.compile_oracle(
        build, "VanillaBedPoiOracle", jars, baking.client_libraries(), lock["minecraft"]["java_major"])
    expected = build / "vanilla.txt"
    oracle_harness.java_run(build, classes, dependencies, "VanillaBedPoiOracle", [expected], "vanilla")
    env = dict(lab.game_env(), HORIZON_BED_POI_ORACLE=str(expected))
    oracle_harness.run_logged([
        "cargo", "test", "--locked", "-p", "horizon-client-lib", "--lib",
        "mobs::villager_brain::poi_classification_tests::vanilla_bed_poi_states", "--", "--include-ignored", "--nocapture",
    ], build, "rust", env=env)
    print(f"{len(expected.read_text().splitlines())} vanilla block states match; runtime and saved bed POI classifiers agree")


if __name__ == "__main__":
    main()
