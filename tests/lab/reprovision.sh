#!/bin/sh
# Run on the disposable development VM, from the repository root.
set -eu
exec python3 "$(dirname "$0")/lab.py" run "$@"
