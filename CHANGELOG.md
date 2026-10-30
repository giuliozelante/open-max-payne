# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- CI matrix (`ubuntu` / `windows` / `macos`) for library crates
- `clippy` + `rustfmt` + `shellcheck` gates
- CodeQL (`rust`, `javascript-typescript`)
- Dependabot (Cargo, Actions, Studio npm)
- Tag release workflow publishing `openras` binaries

## [0.1.0] — 2026-10-30

### Added

- Cargo workspace: `openras`, `maxfx-patcher`, `maxfx-play`, Tauri Studio
- Localized-safe Play stack (any non-English `x_<lang>.ras`)
- HTTPS + SHA-256 pinned 1.05 drop-in apply; SFX installers refused
- Public GitHub repository
