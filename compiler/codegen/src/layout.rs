use std::cell::RefCell;
use std::collections::HashMap;

use nether_llvm::{ModuleCx, StructTy, Ty};
use nether_resolver::{DefId, DefKind, Definitions};
use nether_typecheck::{alloc_kind, AllocKind, PrimitiveKind, Signatures, Type, TypeShape};

/// Every heap-kind `Struct`/`TupleStruct`'s LLVM layout is exactly its
/// declared fields, in declaration order — no refcount header, no
/// hidden fields. `Retain`/`Release` (`crate::function::FnCodegen`) are
/// opaque calls into `runtime/arc`, given only the pointer
/// [`crate::runtime::alloc`] returns; wherever the runtime puts its own
/// bookkeeping is invisible to (and none of the business of) this crate,
/// which only ever addresses a heap object through the fields this
/// struct describes.
pub struct StructLayout<'ctx> {
    pub ty: StructTy<'ctx>,
    /// Declared field order — a GEP index directly.
    pub field_tys: Vec<Ty<'ctx>>,
}

/// An enum's LLVM layout: `{ i64 tag, <every variant's fields, flattened
/// and concatenated> }` — deliberately *not* an overlapping/union layout
/// (see this crate's module docs for why): simple, valid, and correct,
/// at the cost of a struct sized as if every variant's fields coexisted
/// simultaneously. `field_offsets[(variant, index)]` is that field's GEP
/// index into `ty` (offset by 1 for the leading tag field).
pub struct EnumLayout<'ctx> {
    pub ty: StructTy<'ctx>,
    pub field_offsets: HashMap<(u32, u32), u32>,
}

/// Computes and caches every `nether_typecheck::Type`'s LLVM
/// representation, and every heap `Struct`/`TupleStruct`/`Enum`'s field
/// layout — the one place `nether_codegen` decides these, since neither
/// `typecheck`'s `Type` nor `docs/architecture/type-system.md` fix an
/// actual memory layout (only the heap-vs-stack classification).
type EnumLayoutEntry<'ctx> = (StructTy<'ctx>, HashMap<(u32, u32), u32>);

pub struct Layout<'m, 'ctx> {
    m: &'m ModuleCx<'ctx>,
    pub defs: &'m Definitions,
    pub sigs: &'m Signatures,
    // Generic structs need one layout per concrete instantiation.
    structs: RefCell<HashMap<Type, (StructTy<'ctx>, Vec<Ty<'ctx>>)>>,
    // Generic enums need a distinct LLVM layout per concrete
    // instantiation (`Option<i32>` and `Option<String>` do not have the
    // same payload representation).
    enums: RefCell<HashMap<Type, EnumLayoutEntry<'ctx>>>,
}

impl<'m, 'ctx> Layout<'m, 'ctx> {
    pub fn new(m: &'m ModuleCx<'ctx>, defs: &'m Definitions, sigs: &'m Signatures) -> Self {
        Layout { m, defs, sigs, structs: RefCell::new(HashMap::new()), enums: RefCell::new(HashMap::new()) }
    }

    /// The LLVM type a MIR local/value of `ty` is held as — always a
    /// single, non-aggregate-passed-by-value type: `ptr` for anything
    /// heap-kind (language-spec §3.3) or a `weak`/function reference, and
    /// the type's own flattened layout for a stack-kind aggregate (which
    /// `nether_codegen` always addresses through a pointer too — an
    /// `alloca` rather than a heap allocation — never passed around as an
    /// LLVM aggregate SSA value; see this crate's module docs).
    pub fn llvm_type(&self, ty: &Type) -> Ty<'ctx> {
        match ty {
            Type::Primitive(p) => self.primitive_type(*p),
            Type::String | Type::Array(_) | Type::Function(_, _) | Type::Weak(_) => self.m.ptr_type(),
            Type::Struct(_, _) | Type::TupleStruct(_, _) => match alloc_kind(ty, self.defs) {
                AllocKind::Heap => self.m.ptr_type(),
                AllocKind::Stack => self.struct_layout(ty).ty.into(),
            },
            Type::Tuple(elems) => {
                let field_tys: Vec<Ty<'ctx>> = elems.iter().map(|t| self.llvm_type(t)).collect();
                self.m.struct_type(&field_tys).into()
            }
            Type::Enum(_, _) => self.enum_layout(ty).ty.into(),
            Type::Never | Type::Error => self.m.int_type(1),
            Type::Interface(_) | Type::Generic(_) => {
                unreachable!("{ty:?} never appears as a value's type by the time monomorphized MIR reaches codegen")
            }
        }
    }

    pub fn primitive_type(&self, p: PrimitiveKind) -> Ty<'ctx> {
        match p {
            PrimitiveKind::Bool => self.m.bool_type(),
            PrimitiveKind::Char => self.m.int_type(32),
            PrimitiveKind::I8 | PrimitiveKind::U8 => self.m.int_type(8),
            PrimitiveKind::I16 | PrimitiveKind::U16 => self.m.int_type(16),
            PrimitiveKind::I32 | PrimitiveKind::U32 => self.m.int_type(32),
            PrimitiveKind::I64 | PrimitiveKind::U64 => self.m.int_type(64),
            PrimitiveKind::Isize | PrimitiveKind::Usize => self.m.int_type(64),
            PrimitiveKind::F32 => self.m.f32_type(),
            PrimitiveKind::F64 => self.m.f64_type(),
        }
    }

    pub fn is_signed(p: PrimitiveKind) -> bool {
        matches!(p, PrimitiveKind::I8 | PrimitiveKind::I16 | PrimitiveKind::I32 | PrimitiveKind::I64 | PrimitiveKind::Isize)
    }

    pub fn is_float(p: PrimitiveKind) -> bool {
        matches!(p, PrimitiveKind::F32 | PrimitiveKind::F64)
    }

    /// `Struct(id, args)`/`TupleStruct(id, args)`'s own field layout (used both for a
    /// heap type's allocation size/GEP base and a stack type's `alloca`
    /// type) — memoized per concrete `Type` since every use of the same
    /// instantiation must
    /// share one exact LLVM `StructType` (LLVM struct-type identity
    /// matters for `struct_gep`).
    pub fn struct_layout(&self, struct_ty: &Type) -> StructLayout<'ctx> {
        if let Some((ty, field_tys)) = self.structs.borrow().get(struct_ty) {
            return StructLayout { ty: *ty, field_tys: field_tys.clone() };
        }
        let field_tys: Vec<Ty<'ctx>> = self
            .sigs
            .type_fields(struct_ty)
            .unwrap_or_default()
            .iter()
            .map(|field| self.llvm_type(field))
            .collect();
        let ty = self.m.struct_type(&field_tys);
        self.structs
            .borrow_mut()
            .insert(struct_ty.clone(), (ty, field_tys.clone()));
        StructLayout { ty, field_tys }
    }

    /// `Enum(id, _)`'s flattened layout — see [`EnumLayout`]'s docs for
    /// the representation this crate chose.
    pub fn enum_layout(&self, enum_ty: &Type) -> EnumLayout<'ctx> {
        let Type::Enum(id, _) = enum_ty else {
            panic!("enum_layout called with non-enum type {enum_ty:?}");
        };
        if let Some((ty, field_offsets)) = self.enums.borrow().get(enum_ty) {
            return EnumLayout { ty: *ty, field_offsets: field_offsets.clone() };
        }
        let tag_ty = self.m.int_type(64);
        let mut field_tys = vec![tag_ty];
        let mut field_offsets = HashMap::new();
        if let Some(sig) = self.sigs.enum_sigs.get(id) {
            for (variant_idx, _) in sig.variants.iter().enumerate() {
                let payload = self.sigs.enum_payload(enum_ty, variant_idx as u32).unwrap_or_default();
                for (field_idx, field_ty) in payload.iter().enumerate() {
                    field_offsets.insert((variant_idx as u32, field_idx as u32), field_tys.len() as u32);
                    field_tys.push(self.llvm_type(field_ty));
                }
            }
        }
        let ty = self.m.struct_type(&field_tys);
        self.enums.borrow_mut().insert(enum_ty.clone(), (ty, field_offsets.clone()));
        EnumLayout { ty, field_offsets }
    }

    /// Mirrors `nether_mir::build::owner_as_type`'s own reasoning (same
    /// problem, different crate): recovering a method owner's `Type`
    /// variant (`Struct` vs `TupleStruct` vs `Enum`) from only a `DefId`.
    pub fn owner_as_type(&self, id: DefId) -> Type {
        match self.defs.get(id).kind {
            DefKind::Enum => Type::Enum(id, Vec::new()),
            _ => match self.sigs.type_shapes.get(&id) {
                Some(TypeShape::TupleStruct(_)) => Type::TupleStruct(id, Vec::new()),
                _ => Type::Struct(id, Vec::new()),
            },
        }
    }
}
