#!/usr/bin/env python3
"""Reproduce the offline parser archive from a locally downloaded pinned wheel.

Developer tool only; neither CI nor the hook downloads dependencies.
"""
import argparse
import hashlib
from pathlib import Path
import zipfile

SOURCE_URL = "https://files.pythonhosted.org/packages/71/60/917329f640924b18ff085ab889a11c763e0b573da888e8404ff486657602/pyyaml-6.0.3-cp311-cp311-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl"
SOURCE_SHA256 = "b8bb0864c5a28024fac8a632c443c87c5aa6f215c0b126c449ae1a150412f31d"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path, help=f"download separately from {SOURCE_URL}")
    args = parser.parse_args()
    if hashlib.sha256(args.wheel.read_bytes()).hexdigest() != SOURCE_SHA256:
        parser.error("wheel SHA256 does not match the pinned upstream source")
    output = Path(__file__).with_name("vendor") / "pyyaml-6.0.3.zip"
    output.parent.mkdir(exist_ok=True)
    with zipfile.ZipFile(args.wheel) as source, zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as target:
        names = sorted(n for n in source.namelist() if n.startswith("yaml/") and n.endswith(".py"))
        names.append("pyyaml-6.0.3.dist-info/licenses/LICENSE")
        for name in names:
            destination = "LICENSE" if name.endswith("/LICENSE") else name
            info = zipfile.ZipInfo(destination, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            target.writestr(info, source.read(name))
    print(f"{hashlib.sha256(output.read_bytes()).hexdigest()}  {output} ({output.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
