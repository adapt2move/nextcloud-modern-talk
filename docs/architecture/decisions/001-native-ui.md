# Native Rust and GPUI

The desktop uses Rust and GPUI without an embedded WebView. Composition owns application services and the window lifetime; presentation consumes the [bounded core UI interface](../../../crates/talk-core/src/ui.rs). Authentication opens the system browser deliberately. HTML prototypes are design references only.

[rust-toolchain.toml](../../../rust-toolchain.toml), the workspace manifests and [Cargo.lock](../../../Cargo.lock) define the actual compiler, dependency and feature pins. Do not infer implementation from old feasibility notes. Application Rust forbids unsafe code; native bindings encapsulate their upstream FFI. Third-party and bundled asset licenses remain separate from the application license.

The platform-specific build scripts compile GPUI's native renderers. Linux runtime tests and Windows/macOS compile checks have distinct scope. Native permission, device, credential-store, accessibility and performance behavior must be verified on the relevant platform; compilation alone does not establish support. See [development](../../development.md) and [current limitations](../../status.md).
