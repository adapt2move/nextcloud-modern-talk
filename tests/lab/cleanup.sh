#!/bin/sh
# Recovery only: the normal run also cleans up on failure and SIGTERM/SIGINT.
set -eu
exec python3 "$(dirname "$0")/lab.py" cleanup "$@"
