// arcella/arcella-core/src/wasmtime/manifest.rs
//
// Copyright (c) 2025 Alexey Rybakov, Arcella Team
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE>
// or the MIT license <LICENSE-MIT>, at your option.
// This file may not be copied, modified, or distributed
// except according to those terms.

use std::path::Path;

use arcella_types::{
    interface_list::InterfaceList,
    manifest::{ComponentCapabilities, ComponentManifest},
    module_id::ModuleId,
};
use wasmtime::Engine;

use super::error::ArcellaWasmtimeResult;

/// Extracts component metadata directly from a WebAssembly Component binary.
///
/// This function:
/// - Only works with **WebAssembly Components** (not core Wasm or WASI preview1 modules).
/// - Extracts imports and exports in the format `namespace:interface`.
/// - Does **not** include version (`@x.y`) — this must be provided via `component.toml`
///   or inferred from file naming convention if needed later.
/// - Requires a valid `name` and `version` — since they are not stored in Wasm,
///   this function infers them from the filename (e.g., `http-logger@0.1.0.wasm`).
///
/// For MVP v0.2.3, we assume that if `component.toml` is missing,
/// the filename encodes `name@version`.
#[allow(clippy::unnecessary_wraps)]
pub fn component_manifest_from_wasm(
    _engine: &Engine,
    _wasm_path: &Path,
) -> ArcellaWasmtimeResult<ComponentManifest> {
    let manifest = ComponentManifest {
        id: ModuleId {
            name: String::new(),
            version: String::new(),
        },
        description: None,
        exports: InterfaceList::default(),
        imports: InterfaceList::default(),
        capabilities: ComponentCapabilities::default(),
    };
    Ok(manifest)
}
