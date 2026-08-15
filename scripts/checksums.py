#!/usr/bin/env python3
"""Print SHA256SUMS entries for release artifacts."""
import hashlib
import pathlib
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: checksums.py FILE...", file=sys.stderr)
        return 2
    for name in sys.argv[1:]:
        path = pathlib.Path(name)
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        print(f"{digest}  {path.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
