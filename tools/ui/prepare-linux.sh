#!/usr/bin/env bash
# Run only in an assigned disposable development VM. No desktop/backend service starts here.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
# The pinned libwebrtc SDK ships libc++ headers that require Clang 21. Ubuntu
# 24.04 defaults to 18; use LLVM's signed 21 repository when the distro lacks it.
candidate=$(apt-cache policy clang-21 | awk '$1 == "Candidate:" { print $2 }')
if [[ -z "$candidate" || "$candidate" == "(none)" ]]; then
  . /etc/os-release
  if [[ "$ID" != ubuntu || "$VERSION_CODENAME" != noble ]]; then
    echo "Install Clang 21 for this distribution before running this script." >&2
    exit 1
  fi
  apt-get install -y --no-install-recommends ca-certificates curl gnupg
  install -d -m 0755 /etc/apt/keyrings
  key=$(mktemp)
  trap 'rm -f "$key"' EXIT
  curl --fail --silent --show-error --max-time 60 https://apt.llvm.org/llvm-snapshot.gpg.key -o "$key"
  fingerprint=$(gpg --batch --show-keys --with-colons "$key" | awk -F: '$1 == "fpr" { print $10; exit }')
  [[ "$fingerprint" == 6084F3CF814B57C1CF12EFD515CF4D18AF4F7421 ]]
  install -m 0644 "$key" /etc/apt/keyrings/llvm.asc
  printf '%s\n' 'deb [signed-by=/etc/apt/keyrings/llvm.asc] https://apt.llvm.org/noble/ llvm-toolchain-noble-21 main' > /etc/apt/sources.list.d/llvm-21.list
  apt-get update
fi
apt-get install -y --no-install-recommends build-essential pkg-config clang-21 lld-21 libclang-21-dev \
  libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libfontconfig1-dev libfreetype6-dev libssl-dev libvulkan1 mesa-vulkan-drivers \
  vulkan-tools xvfb xauth xdotool imagemagick fonts-noto-core fonts-noto-color-emoji \
  libnotify-bin pulseaudio-utils dbus-x11 x11-utils tesseract-ocr tesseract-ocr-deu xclip \
  xdg-desktop-portal xdg-desktop-portal-gtk gsettings-desktop-schemas \
  libasound2-dev libpulse-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-tools
# Camera previews require GStreamer >= 1.24. The production provider uses native
# libraries; gst-inspect is installed only to diagnose this development VM.
