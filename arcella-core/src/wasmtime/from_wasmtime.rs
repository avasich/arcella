// arcella/arcella-core/src/wasmtime/from_wasmtime.rs
//
// Copyright (c) 2025 Alexey Rybakov, Arcella Team
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE>
// or the MIT license <LICENSE-MIT>, at your option.
// This file may not be copied, modified, or distributed
// except according to those terms.

//! Conversion utilities from `wasmtime::component::types` to `arcella_types::spec`.

use std::collections::HashMap;

use arcella_types::spec::ComponentItemSpec;
use wasmtime::{
    Engine,
    component::types::{self, ComponentExtern, ComponentItem},
};

use super::ArcellaWasmtimeResult;

const MAX_RECURSION_DEPTH: usize = 32;

/// Extension trait to convert `wasmtime::component::types::ComponentItem` into `ComponentItemSpec`.
pub trait ComponentItemSpecExt {
    /// Converts a `ComponentItem` into a serializable `ComponentItemSpec`.
    ///
    /// This is a best-effort, lossy conversion suitable for introspection and manifest generation.
    /// Full type fidelity requires integration with `wit-parser` (planned for v0.4+).
    ///
    /// # Arguments
    ///
    /// * `engine` - The Wasmtime engine used to inspect component types.
    ///
    /// # Returns
    ///
    /// A `ComponentItemSpec` representing the item, or an error on failure.
    fn to_spec(&self, engine: &Engine) -> ArcellaWasmtimeResult<ComponentItemSpec>;
}

impl ComponentItemSpecExt for ComponentExtern<'_> {
    fn to_spec(&self, engine: &Engine) -> ArcellaWasmtimeResult<ComponentItemSpec> {
        todo!("move into engine");
    }
}

/// Extension trait for `wasmtime::component::types::Component`.
pub trait ComponentTypeExt {
    /// Extracts imports as a map of `ComponentItemSpec`.
    fn imports_spec(
        &self,
        engine: &Engine,
    ) -> ArcellaWasmtimeResult<HashMap<String, ComponentItemSpec>>;

    /// Extracts exports as a map of `ComponentItemSpec`.
    fn exports_spec(
        &self,
        engine: &Engine,
    ) -> ArcellaWasmtimeResult<HashMap<String, ComponentItemSpec>>;
}

impl ComponentTypeExt for types::Component {
    fn imports_spec(
        &self,
        engine: &Engine,
    ) -> ArcellaWasmtimeResult<HashMap<String, ComponentItemSpec>> {
        self.imports(engine).map(|(name, item)| Ok((name.into(), item.to_spec(engine)?))).collect()
    }

    fn exports_spec(
        &self,
        engine: &Engine,
    ) -> ArcellaWasmtimeResult<HashMap<String, ComponentItemSpec>> {
        self.exports(engine).map(|(name, item)| Ok((name.into(), item.to_spec(engine)?))).collect()
    }
}

fn to_spec_with_depth(
    item: &ComponentExtern<'_>,
    engine: &Engine,
    depth: usize,
    max_depth: usize,
) -> ArcellaWasmtimeResult<ComponentItemSpec> {
    if depth > max_depth {
        return Ok(ComponentItemSpec::Unknown {
            debug: Some("Exceeded maximum recursion depth".into()),
        });
    }

    match &item.ty {
        ComponentItem::ComponentFunc(func_ty) => {
            let params =
                func_ty.params().map(|(name, ty)| (name.into(), type_to_string(&ty))).collect();
            let results = func_ty.results().map(|ty| type_to_string(&ty)).collect();
            Ok(ComponentItemSpec::ComponentFunc { params, results })
        },

        ComponentItem::CoreFunc(ty) => Ok(ComponentItemSpec::CoreFunc(format!("{ty}"))),

        ComponentItem::Module(ty) => Ok(ComponentItemSpec::Module(format!("{ty:?}"))),

        ComponentItem::Component(comp_ty) => {
            let imports = comp_ty
                .imports(engine)
                .map(|(name, nested_item)| {
                    (
                        name.into(),
                        match to_spec_with_depth(&nested_item, engine, depth + 1, max_depth) {
                            Ok(item) => item,
                            // Best-effort parsing: skip malformed nested items
                            Err(e) => ComponentItemSpec::Unknown {
                                debug: Some(format!("Error: {e:?}")),
                            },
                        },
                    )
                })
                .collect();
            let exports = comp_ty
                .exports(engine)
                .map(|(name, nested_item)| {
                    (
                        name.into(),
                        match to_spec_with_depth(&nested_item, engine, depth + 1, max_depth) {
                            Ok(item) => item,
                            // Best-effort parsing: skip malformed nested items
                            Err(e) => ComponentItemSpec::Unknown {
                                debug: Some(format!("Error: {e:?}")),
                            },
                        },
                    )
                })
                .collect();
            Ok(ComponentItemSpec::Component { imports, exports })
        },

        ComponentItem::ComponentInstance(ty) => {
            let exports = ty
                .exports(engine)
                .map(|(name, nested_item)| {
                    (
                        name.into(),
                        match to_spec_with_depth(&nested_item, engine, depth + 1, max_depth) {
                            Ok(item) => item,
                            // Best-effort parsing: skip malformed nested items
                            Err(e) => ComponentItemSpec::Unknown {
                                debug: Some(format!("Error: {e:?}")),
                            },
                        },
                    )
                })
                .collect();
            Ok(ComponentItemSpec::ComponentInstance { exports })
        },

        ComponentItem::Type(ty) => {
            // TODO(v0.4): Replace with WIT type name via `wit-parser` or canonical string
            Ok(ComponentItemSpec::Type(format!("{ty:?}")))
        },

        ComponentItem::Resource(ty) => {
            // TODO(v0.4): Replace with WIT type name via `wit-parser` or canonical string
            Ok(ComponentItemSpec::Resource(format!("{ty:?}")))
        },
    }
}

fn type_to_string(ty: &types::Type) -> String {
    match ty {
        types::Type::Bool => "bool".into(),
        types::Type::S8 => "s8".into(),
        types::Type::U8 => "u8".into(),
        types::Type::S16 => "s16".into(),
        types::Type::U16 => "u16".into(),
        types::Type::S32 => "s32".into(),
        types::Type::U32 => "u32".into(),
        types::Type::S64 => "s64".into(),
        types::Type::U64 => "u64".into(),
        types::Type::Float32 => "f32".into(),
        types::Type::Float64 => "f64".into(),
        types::Type::Char => "char".into(),
        types::Type::String => "string".into(),
        _ => format!("unknown({ty:?})"),
    }
}

#[cfg(test)]
mod tests {}
