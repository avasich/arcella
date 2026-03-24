// arcella/arcella-core/src/utils/fs/mod.rs
//
// Copyright (c) 2025 Alexey Rybakov, Arcella Team
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE>
// or the MIT license <LICENSE-MIT>, at your option.
// This file may not be copied, modified, or distributed
// except according to those terms.

//! File system and TOML configuration utilities for Arcella.
//!
//! This crate provides common functions for:
//! - Resolving base directories based on executable location or environment.
//! - Finding and validating `.toml` configuration files.
//! - Collecting files specified by `includes` patterns.
//! - Converting TOML values into a serializable format used by Arcella.
//! - Loading configurations recursively with warnings.
//!
//! It is designed to be used by the Arcella runtime and other tools that need
//! to process TOML-based configurations in a consistent way.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use tokio::fs;
use uuid::Uuid;

use super::error::{ArcellaUtilsError, ArcellaUtilsResult};

#[derive(Debug)]
pub struct AppDirs {
    pub base: PathBuf,
    pub config: PathBuf,
}

/// Returns the base and configuration directories for the application.
///
/// Checks the following locations in order, returning the first match:
///
/// 1. **Exe-relative** – if `exe_path` is inside `<root>/` or `<root>/bin/`
///    and `<root>/config/` exists: base = `<root>`, config = `<root>/config/`.
///    `/bin/` directly under the filesystem root is excluded.
/// 2. **XDG / platform dirs** – config = `{config_dir}/arcella/`,
///    base = `{data_dir}/arcella/` with `{home}/arcella/` as a fallback.
/// 3. **Home dir fallback** – base = `{home}/.arcella/`, config = `{home}/.arcella/config/`.
///    Used only if the platform config dir cannot be determined.
///
/// Options 2 and 3 are not checked for existence.
///
/// # Examples
///
/// ```
/// // /opt/arcella/bin/exe + /opt/arcella/config/ exists
/// //   -> base:   /opt/arcella,
/// //   -> config: /opt/arcella/config
/// // config_dir and data_dir available
/// //   -> base:   ~/.local/share/arcella
/// //   -> config: ~/.config/arcella
/// // config_dir available, data_dir not
/// //   -> base:   ~/.arcella,
/// //   -> config: ~/.config/arcella
/// // config_dir is not available
/// //   -> base:   ~/.arcella
/// //   -> config: ~/.arcella/config
/// ```
pub fn get_app_dirs(exe_path: Option<&impl AsRef<Path>>) -> ArcellaUtilsResult<AppDirs> {
    exe_path
        .and_then(|exe| {
            let exe_dir = exe.as_ref().parent().filter(|p| !p.as_os_str().is_empty())?;

            let base = if exe_dir.ends_with("bin") {
                let grandparent = exe_dir.parent()?;
                grandparent.parent()?;
                grandparent
            } else {
                exe_dir
            };

            let config = base.join("config");
            config.is_dir().then_some(AppDirs {
                base: base.to_path_buf(),
                config,
            })
        })
        .or_else(|| {
            let home_base = || dirs::home_dir().map(|h| h.join(".arcella"));
            let config = dirs::config_dir().map(|dir| dir.join("arcella"));

            if let Some(config) = config {
                let base = dirs::data_dir().map(|dir| dir.join("arcella")).or_else(home_base)?;
                Some(AppDirs { base, config })
            } else {
                let base = home_base()?;
                Some(AppDirs {
                    config: base.join("config"),
                    base,
                })
            }
        })
        .ok_or_else(|| ArcellaUtilsError::Internal("Cannot determine app directories".into()))
}


/// Creates a crash-safe temporary subdirectory with a unique name.
///
/// The directory name follows the pattern:
/// - `{prefix}.tmp-{uuid}` if `prefix` is provided,
/// - `tmp-{uuid}` otherwise.
///
/// The UUID (v4) ensures global uniqueness across processes and reboots.
/// Invalid characters in `prefix` are replaced with underscores to ensure portability.
///
/// # Safety
///
/// The created directory is empty and owned by the current user.
/// Caller is responsible for cleanup (e.g., via `tempfile::TempDir` semantics or manual `remove_dir_all`).
///
/// # Errors
///
/// Fails if:
/// - `parent_dir` does not exist or is not writable,
/// - OS fails to create the directory (e.g., too long path, permission denied).
pub async fn create_temp_subdir(
    parent_dir: &Path,
    prefix: Option<&str>,
) -> ArcellaUtilsResult<PathBuf> {
    let uuid_part = Uuid::new_v4().simple();

    let temp_name = prefix.map_or_else(
        || format!("tmp-{uuid_part}"),
        |p| {
            // Sanitize prefix to filesystem-safe characters
            let clean_prefix =
                p.replace(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '-'), "_");
            format!("{clean_prefix}.tmp-{uuid_part}")
        },
    );

    let temp_path = parent_dir.join(temp_name);

    tokio::fs::create_dir_all(&temp_path).await.map_err(|e| ArcellaUtilsError::IoWithPath {
        source: e,
        path: temp_path.clone(),
    })?;

    Ok(temp_path)
}

/// Copies multiple files to a target directory in parallel.
///
/// Preserves original filenames. Optionally ensures **durability** via `fsync`.
///
/// ## When to use `sync = true`
/// - **Final installation** into `modules/` — to guarantee module integrity after crash.
/// - **Critical config updates** — where partial write must never be visible.
///
/// ## When to use `sync = false`
/// - **Staging/temporary copies** (e.g., before validation or rename).
/// - **Performance-sensitive bulk operations** where crash recovery is acceptable.
///
/// # Contract
/// - **Precondition**: `target_dir` must exist and be a writable directory.
/// - **Input**: All `files` must exist and be readable.
/// - **Output**: Returns paths to copied files (same order as input).
/// - **Failure**: Partial copies may remain; caller is responsible for cleanup.
///
/// # Errors
/// - `InvalidArgument` if any source file has no name or `target_dir` is not a directory.
/// - `IoWithPath` on I/O errors (not found, permission denied, disk full, etc.).
pub async fn copy_files_to_dir(
    files: &[PathBuf],
    target_dir: &Path,
    sync: bool,
) -> ArcellaUtilsResult<Vec<PathBuf>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }

    // Validate target directory upfront
    if !target_dir.is_dir() {
        return Err(ArcellaUtilsError::InvalidArgument {
            message: format!("Target is not a directory: '{}'", target_dir.display()),
        });
    }

    // Parallel copy phase
    let copy_futures: Vec<_> = files
        .iter()
        .map(|src_path| {
            let target_dir = target_dir.to_path_buf();
            async move {
                let file_name =
                    src_path.file_name().ok_or_else(|| ArcellaUtilsError::InvalidArgument {
                        message: format!("File has no name: '{}'", src_path.display()),
                    })?;

                let dest_path = target_dir.join(file_name);

                fs::copy(src_path, &dest_path).await.map_err(|e| {
                    ArcellaUtilsError::IoWithPath {
                        source: e,
                        path: src_path.clone(),
                    }
                })?;

                let result: ArcellaUtilsResult<PathBuf> = Ok(dest_path);
                result
            }
        })
        .collect();

    let staged_paths = futures::future::try_join_all(copy_futures).await?;

    // Optional: ensure all file data is on persistent storage
    if sync {
        let file_sync_futures: Vec<_> = staged_paths
            .iter()
            .map(|path| async {
                let path = path.clone();
                let file = tokio::fs::File::open(path).await?;
                file.sync_all().await?;
                Ok::<(), ArcellaUtilsError>(())
            })
            .collect();

        futures::future::try_join_all(file_sync_futures).await?;
    }

    Ok(staged_paths)
}

/// Synchronizes a directory’s metadata to persistent storage.
///
/// On Unix-like systems, this calls `fsync()` on the directory file descriptor,
/// which ensures that **directory entries** (i.e., filenames and their inode links)
/// are committed to disk. This is critical after creating or renaming files.
///
/// On Windows, this is a no-op (directory fsync is not supported in standard APIs),
/// but the function remains for cross-platform compatibility.
///
/// # Why this matters
///
/// After writing files into a directory, a crash could leave the directory
/// in a state where files exist on disk but are not linked by name.
/// `fsync` on the directory prevents this.
///
/// # Errors
///
/// - `IoWithPath`: if the directory cannot be opened or synced.
pub async fn sync_directory(dir: &Path) -> ArcellaUtilsResult<()> {
    let file = fs::File::open(dir).await.map_err(|e| ArcellaUtilsError::IoWithPath {
        source: e,
        path: dir.to_path_buf(),
    })?;
    file.sync_all().await.map_err(|e| ArcellaUtilsError::IoWithPath {
        source: e,
        path: dir.to_path_buf(),
    })?;
    Ok(())
}


/// Maximum allowed length for base names (e.g., module name, deployment ID).
/// Chosen to prevent filesystem path overflow and ensure readability.
const MAX_BASE_NAME_LENGTH: usize = 128;

/// Extracts the base name from a file path by stripping a **required** extension.
///
/// Examples:
/// - `"/x/app.wasm"` → `"app"` (if ext = `"wasm"`)
/// - `"web.deployment.toml"` → `"web"` (if ext = `"deployment.toml"`)
///
/// # Errors
/// - If file has no name
/// - If extension does not match exactly
pub fn base_name_strip_ext(path: impl AsRef<Path>, ext: &str) -> ArcellaUtilsResult<String> {
    let file_name = path.as_ref().file_name().and_then(OsStr::to_str).ok_or_else(|| {
        ArcellaUtilsError::InvalidArgument {
            message: "File has no valid UTF-8 name".into(),
        }
    })?;
    file_name
        .strip_suffix(ext)
        .and_then(|prefix| prefix.strip_suffix("."))
        .filter(|prefix| !prefix.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ArcellaUtilsError::InvalidArgument {
            message: format!("File '{file_name}' does not have extension '{ext}'"),
        })
}

/// Validates that a base name (e.g., module name, deployment ID) contains only safe characters.
///
/// Allowed: ASCII letters, digits, hyphens (`-`), underscores (`_`).
/// This matches the `ModuleId::is_valid_name` rule.
pub fn validate_base_name(name: &str) -> ArcellaUtilsResult<()> {
    if name.is_empty() || name.len() > MAX_BASE_NAME_LENGTH {
        return Err(ArcellaUtilsError::InvalidArgument {
            message: "Base name is empty or too long".into(),
        });
    }

    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(ArcellaUtilsError::InvalidArgument {
            message: "Base name contains invalid characters (allowed: a-z, A-Z, 0-9, -, _)".into(),
        });
    }

    if name.starts_with('-') || name.ends_with('-') {
        return Err(ArcellaUtilsError::InvalidArgument {
            message: "Base name cannot start or end with hyphen".into(),
        });
    }

    Ok(())
}

// /// Constructs a sibling file path by appending a suffix to the file stem.
// ///
// /// Examples:
// /// - `"app.wasm"` + `".component.toml"` → `"app.component.toml"`
// /// - `".env.wasm"` + `".toml"` → `".env.toml"`
// ///
// /// If the input has no stem (e.g., `/` or `.`), the result is undefined
// /// and should not be relied upon. In practice, callers ensure valid paths.
// pub fn sibling_path_with_suffix<P: AsRef<Path>>(original: P, suffix: &str) -> PathBuf {
//     let original = original.as_ref();
//     let stem = original.file_stem().or_else(|| original.file_name()).unwrap_or_default();
//     original.with_file_name(format!("{}{}", stem.to_string_lossy(), suffix))
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_name_strip_ext() {
        assert_eq!(base_name_strip_ext("app.wasm", "wasm").unwrap(), "app");
        assert_eq!(
            base_name_strip_ext("hello-world@1.0.0.deployment.toml", "deployment.toml").unwrap(),
            "hello-world@1.0.0"
        );
        assert!(base_name_strip_ext("bad.txt", "wasm").is_err());
        assert!(base_name_strip_ext(".wasm", "wasm").is_err());
    }

    #[test]
    fn test_validate_base_name() {
        assert!(validate_base_name("valid_mod").is_ok());
        assert!(validate_base_name("test-123").is_ok());
        assert!(validate_base_name("").is_err());
        assert!(validate_base_name("invalid name!").is_err());
        assert!(validate_base_name(&"x".repeat(129)).is_err());
    }
}
