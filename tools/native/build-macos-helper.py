#!/usr/bin/env python3
"""Compile the app-bundled helper with Apple's SDK; never launch it or modify CI."""
import argparse
import platform
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if platform.system() != 'Darwin' or platform.machine() != 'arm64':
    parser.error('requires an Apple Silicon macOS runner with the selected Xcode SDK')
source = Path(__file__).resolve().parents[2] / 'native/macos/TalkNativeHelper.swift'
args.output.parent.mkdir(parents=True, exist_ok=True)
subprocess.run(['xcrun', '--sdk', 'macosx', 'swiftc', '-swift-version', '5', '-O',
                '-target', 'arm64-apple-macosx15.7', '-framework', 'AppKit',
                '-framework', 'UserNotifications', '-framework', 'AVFoundation',
                '-framework', 'CoreGraphics', str(source), '-o', str(args.output)], check=True)
