# openras (Rust)

Clean-room RAS/MPM reader/writer for Max Payne 1. Spec: [`docs/specs/ras_format.md`](../../docs/specs/ras_format.md).

```bash
cargo run -p openras -- list archive.mpm
cargo run -p openras -- extract archive.mpm -d ./out
cargo run -p openras -- pack ./folder out.mpm
cargo test -p openras
```

Tests use **synthetic** archives only (no binary fixtures in git).
