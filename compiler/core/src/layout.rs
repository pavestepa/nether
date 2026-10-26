//! Target layout validation, independent of source code generation.
use crate::hir::Type;
use nether_semantics::{AggregateLayout, EnumLayout, Layout};
pub fn layout(kind: &Type) -> Result<Layout, String> {
    Ok(match kind {
        Type::Enum { .. } => enum_layout(kind)?.layout,
        Type::Pointer { .. } | Type::Function { .. } | Type::Class { .. } => Layout::POINTER,
        Type::StoredView(_) => {
            return Err("borrowed descriptor layout requires backend context".into())
        }
        Type::Unit => Layout::UNIT,
        Type::Bool => Layout::BOOL,
        Type::Char => Layout::CHAR,
        Type::Integer(i) => Layout::integer(*i),
        Type::F32 => Layout::F32,
        Type::F64 => Layout::F64,
        Type::Str => Layout::STR,
        Type::Array(element, n) => layout(element)?
            .array(*n)
            .map_err(|e| format!("invalid array layout: {e:?}"))?,
        Type::Record { fields, .. } => {
            AggregateLayout::new(
                &fields
                    .iter()
                    .map(|f| layout(&f.ty))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|e| format!("invalid struct layout: {e:?}"))?
            .layout
        }
        Type::Tuple(fields) => {
            AggregateLayout::new(&fields.iter().map(layout).collect::<Result<Vec<_>, _>>()?)
                .map_err(|e| format!("invalid tuple layout: {e:?}"))?
                .layout
        }
    })
}

fn enum_layout(ty: &Type) -> Result<EnumLayout, String> {
    let Type::Enum { variants, .. } = ty else {
        return Err("enum layout requires enum".into());
    };
    let payloads = variants
        .iter()
        .map(|v| layout(&Type::Tuple(v.fields.clone())))
        .collect::<Result<Vec<_>, _>>()?;
    EnumLayout::new(&payloads).map_err(|e| format!("invalid enum layout: {e:?}"))
}
