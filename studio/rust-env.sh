# Bash only. From fish/zsh run: ./build.sh
# Or: bash -c "source rust-env.sh && bun run tauri build"
STABLE="${HOME}/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin"
export RUSTC="${STABLE}/rustc"
export CARGO="${STABLE}/cargo"
export PATH="${STABLE}:${HOME}/.cargo/bin:${PATH}"
unset RUSTUP_TOOLCHAIN
export RUSTUP_FORCE_ARG0=cargo
