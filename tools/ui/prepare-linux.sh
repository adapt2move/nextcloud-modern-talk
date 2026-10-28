#!/usr/bin/env bash
# Run only in an assigned disposable development VM. No desktop/backend service starts here.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends build-essential pkg-config clang lld libclang-dev ca-certificates \
  libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libfontconfig1-dev libfreetype6-dev libssl-dev libvulkan1 mesa-vulkan-drivers \
  vulkan-tools xvfb xauth xdotool imagemagick fonts-noto-core fonts-noto-color-emoji \
  libnotify-bin pulseaudio-utils dbus-x11 x11-utils tesseract-ocr tesseract-ocr-deu xclip \
  xdg-desktop-portal xdg-desktop-portal-gtk gsettings-desktop-schemas \
  libasound2-dev libpulse-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-tools
# Camera previews and the VP8/Opus codecs require GStreamer >= 1.24.
# gst-inspect is installed only to diagnose this development VM.
