# Fish: `source rust-env.fish` before `bun run tauri build`
# Prefer: `./build.sh` (no sourcing needed).
set -gx STABLE "$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin"
set -gx RUSTC "$STABLE/rustc"
set -gx CARGO "$STABLE/cargo"
set -gx PATH $STABLE $HOME/.cargo/bin $PATH
set -e RUSTUP_TOOLCHAIN
set -gx RUSTUP_FORCE_ARG0 cargo
