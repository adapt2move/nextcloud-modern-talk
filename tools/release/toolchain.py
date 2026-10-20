#!/usr/bin/env python3
"""Install exactly rust-toolchain.toml; never resolve or rewrite Cargo.lock."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tomllib

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--target")
a = p.parse_args()
config = tomllib.loads(Path("rust-toolchain.toml").read_text())["toolchain"]
channel = config["channel"]
if not re.fullmatch(r"\d+\.\d+\.\d+", channel):
    raise SystemExit("Exact toolchain required")
command = ["rustup", "toolchain", "install", channel, "--profile", config["profile"]]
for component in config["components"]:
    command += ["--component", component]
if a.target:
    command += ["--target", a.target]
subprocess.run(command, check=True, timeout=300)
if os.environ.get("GITHUB_ENV"):
    with open(os.environ["GITHUB_ENV"], "a") as output:
        output.write("RUSTUP_TOOLCHAIN=" + channel + "\n")
for tool in ("rustc", "cargo"):
    subprocess.run([tool, "+" + channel, "--version"], check=True, timeout=30)
