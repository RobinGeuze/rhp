//! Backing state for the `ZipArchive` class — reading & extracting `.zip`
//! archives (Composer's dist downloader). Pure-Rust via the `zip` crate.
//!
//! The VM owns a map of open archives (id -> `ZipState`); the PHP-level
//! `ZipArchive` prelude class holds the id and delegates to `__zip_*` builtins,
//! which call the Host, which calls these methods.

use std::io::{Cursor, Read};
use std::path::Path;

/// One open zip archive, read fully into memory.
pub struct ZipState {
    archive: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl ZipState {
    /// Parse the archive from its raw bytes. `None` if it is not a valid zip.
    pub fn open(bytes: Vec<u8>) -> Option<ZipState> {
        zip::ZipArchive::new(Cursor::new(bytes))
            .ok()
            .map(|archive| ZipState { archive })
    }

    pub fn count(&self) -> usize {
        self.archive.len()
    }

    /// Entry name at index `i` (php `getNameIndex`).
    pub fn name_index(&mut self, i: usize) -> Option<Vec<u8>> {
        self.archive
            .by_index(i)
            .ok()
            .map(|f| f.name().as_bytes().to_vec())
    }

    /// `(name, uncompressed_size, compressed_size)` for `statIndex`.
    pub fn stat_index(&mut self, i: usize) -> Option<(Vec<u8>, u64, u64)> {
        self.archive
            .by_index(i)
            .ok()
            .map(|f| (f.name().as_bytes().to_vec(), f.size(), f.compressed_size()))
    }

    /// Whole-entry contents by name (php `getFromName`).
    pub fn get_from_name(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let name = std::str::from_utf8(name).ok()?;
        let mut f = self.archive.by_name(name).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    /// Extract every entry under `dest` (php `extractTo`). Returns false on any
    /// I/O error. Entry paths are sanitized against path traversal.
    pub fn extract_to(&mut self, dest: &[u8]) -> bool {
        let dest = match std::str::from_utf8(dest) {
            Ok(s) => Path::new(s),
            Err(_) => return false,
        };
        for i in 0..self.archive.len() {
            let mut entry = match self.archive.by_index(i) {
                Ok(e) => e,
                Err(_) => return false,
            };
            // `enclosed_name` rejects absolute paths and `..` traversal.
            let rel = match entry.enclosed_name() {
                Some(p) => p,
                None => continue,
            };
            let out = dest.join(&rel);
            if entry.is_dir() {
                if std::fs::create_dir_all(&out).is_err() {
                    return false;
                }
                continue;
            }
            if let Some(parent) = out.parent() {
                if std::fs::create_dir_all(parent).is_err() {
                    return false;
                }
            }
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_err() || std::fs::write(&out, &buf).is_err() {
                return false;
            }
            // Preserve unix mode bits where present (executables, etc.).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
                }
            }
        }
        true
    }
}
