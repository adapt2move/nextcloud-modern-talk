# Native UI dependency provenance

The desktop uses **GPUI 0.2.2** with a source-bound local backdrop-renderer patch,
default features disabled, `font-kit` and `x11`, plus
**unicode-segmentation 1.12.0**. Exact versions and
features live in the workspace manifests and `Cargo.lock`; the toolchain is
pinned in `rust-toolchain.toml`.

## Primary sources and exact artifacts

| Material | Pin / provenance | License |
| --- | --- | --- |
| GPUI | [published 0.2.2](https://docs.rs/crate/gpui/0.2.2), [immutable crate archive](https://static.crates.io/crates/gpui/gpui-0.2.2.crate), SHA-256 `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707` | Apache-2.0; full original license bundled |
| Native input adaptation | `examples/input.rs` in that exact archive; selection/edit/IME/render API inspected against archive source | Apache-2.0, attribution and modifications in desktop NOTICE/source |
| Unicode segmentation | [official 1.12.0 documentation](https://docs.rs/unicode-segmentation/1.12.0/unicode_segmentation/) and canonical registry checksum | MIT OR Apache-2.0 |
| Runtime Google Sans | [v14.000 release](https://github.com/googlefonts/googlesans/releases/tag/v14.000); unchanged static Regular, Medium and Bold; [exact archive/file hashes](../../../apps/desktop/assets/fonts/googlesans-provenance.json) | OFL-1.1, with upstream license and trademark guidance bundled |
| Manrope in historical design assets | [Google Fonts commit b31870aff700ab7a1d74fa0c6887d95beb9e0037](https://github.com/google/fonts/tree/b31870aff700ab7a1d74fa0c6887d95beb9e0037/ofl/manrope), SHA-256 `3ae11c49db0455a3cc33e37d380f20fdb8c7f8b41dc07625c177e3d87a9d6ae6` | OFL-1.1; unchanged original notice SHA-256 `58172e0c0fac2cda8a37b348164bb55e44b0e69051e557e92b1d3f6910141f7b` |

GPUI's archive VCS metadata names `69e2130295c2649963eb639fc70b4f2ee8ea1624`
with **dirty=true**. Therefore the published archive/checksum is the implementation
base; it must not be described as an identical clean Git commit.
[UPSTREAM.json](../../../vendor/gpui/UPSTREAM.json) records the unchanged original
inventory and exact local delta; the architecture and packaging checks verify both.
Unpinned Zed main is not substituted. GPUI's source/build manifest selects Metal on macOS and Blade/
Vulkan for the exercised Linux X11 path. Wayland, `macos-blade`, runtime shader
switches and GPUI test-support are not enabled by the desktop manifest.

GPUI is a substantial dependency. Upstream transitive HTTP packages do not grant
presentation modules network access. The architecture check enforces the
application boundary independently of the toolkit's dependency graph.

`tools/ui/dependency-evidence.py` can record a target's resolved closure and verify
cached crate checksums. Release packaging instead collects the shipping desktop's
normal/build dependency closure and its license texts into the package. Do not
reuse an older closure report after changing the manifest or lock. Native distro
libraries and macOS frameworks have separate runtime/distribution requirements.

## Platform prerequisites and limits

The Linux build and capture prerequisite list is in `tools/ui/prepare-linux.sh`:
C/C++ compiler, clang/libclang, pkg-config, X11/XCB/xkbcommon, fontconfig/freetype,
OpenSSL and Vulkan/Mesa. Xvfb, xdotool, ImageMagick, Vulkan tools, Tesseract and
fallback fonts are capture/interaction tools. The bundled Google Sans fonts are
unmodified release files; no font-generation step is required.
No machine-specific font, asset, credential or development path is compiled in.

macOS needs an appropriate Xcode/Metal SDK environment; GPUI's build script invokes
its Metal and dispatch binding toolchain. CI compiles on macOS; Linux software
Vulkan and a successful Mac compilation do not qualify Metal runtime rendering,
permissions, VoiceOver or Keychain.

The native input adaptation and fonts retain their original licenses. `talk-desktop
--licenses` prints the embedded project/input/font notices without opening a window.
Release packaging must also carry applicable dependency notices; see
[apps/desktop/NOTICE.md](../../../apps/desktop/NOTICE.md).
