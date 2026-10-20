#!/usr/bin/env python3
"""Print SHA-256 and size of every file in an output directory and enforce a byte bound.

Reads files only: no subprocess, network, build, test or launch. Works the same
on Linux, Windows and macOS runners.
"""
import argparse
import hashlib
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--max-bytes", type=int, required=True)
    args = parser.parse_args()
    if args.directory.is_symlink() or not args.directory.is_dir():
        raise SystemExit("Output directory missing: " + str(args.directory))
    total = files = 0
    for path in sorted(args.directory.rglob("*")):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise SystemExit("Links and special files are not accepted in outputs: " + str(path))
        if path.is_file():
            size = path.stat().st_size
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            print(f"{digest}  {size:>12}  {path.relative_to(args.directory).as_posix()}")
            total += size
            files += 1
    print(f"{files} files, {total} bytes; bound {args.max_bytes} bytes", flush=True)
    if files == 0 or total > args.max_bytes:
        raise SystemExit("Output is empty or exceeds its byte bound")


if __name__ == "__main__":
    main()
