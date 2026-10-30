//! Clean-room Remedy Archive System (RAS/MPM) for Max Payne 1.
//! Spec: docs/specs/ras_format.md

pub mod archive;
pub mod crypto;
pub mod lzss;
pub mod pathutil;

pub use archive::{RasArchive, RasDirEntry, RasError, RasFileEntry, MODE_LZSS, MODE_STORED};
