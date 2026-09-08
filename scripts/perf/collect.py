#!/usr/bin/env python3
"""Collect public JSON measurements without ROMs, fixtures, or machine identifiers."""
import argparse
import json
from pathlib import Path


def public_metadata(value):
    if isinstance(value, dict):
        return {key: public_metadata(item) for key, item in value.items()
                if not any(word in key.lower() for word in ("serial", "uuid", "udid"))}
    if isinstance(value, list):
        return [public_metadata(item) for item in value]
    return value


def collect(source):
    metadata = public_metadata(json.loads((source / "metadata.json").read_text()))
    runs = []
    for path in sorted(source.glob("*/metrics.json")):
        run = {"run": path.parent.name, "metrics": json.loads(path.read_text()),
               "resources": json.loads(path.with_name("resources.json").read_text()),
               "samples": json.loads(path.with_name("samples.json").read_text())}
        for extra in ("control", "inputs", "foreground"):
            artifact = path.with_name(f"{extra}.json")
            if artifact.exists():
                run[extra] = public_metadata(json.loads(artifact.read_text()))
        runs.append(run)
    return {"metadata": metadata, "runs": runs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    with args.destination.open("x") as output:
        json.dump(collect(args.source), output, separators=(",", ":"))
        output.write("\n")


if __name__ == "__main__":
    main()
