//! Compiler-owned module. Private symbols cannot be spelled by source programs.
pub const MODULE: &str = "core/intrinsic";
pub const SOURCE: &str = include_str!("intrinsic.nr");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    WriteOwned,
    ReadOwned,
    Detach,
    Adopt,
    DropInPlace,
    SlotAddress,
    SlotAddressMut,
    CheckIndex,
    View,
    ViewMut,
    DropRange,
    MoveSlots,
    CheckRange,
    Allocate,
    Initialize,
    Take,
    Replace,
    DropSlot,
    Release,
    Size,
    Align,
}
impl Intrinsic {
    pub const ALL: [Self; 21] = [
        Self::WriteOwned,
        Self::ReadOwned,
        Self::Detach,
        Self::Adopt,
        Self::DropInPlace,
        Self::SlotAddress,
        Self::SlotAddressMut,
        Self::CheckIndex,
        Self::View,
        Self::ViewMut,
        Self::DropRange,
        Self::MoveSlots,
        Self::CheckRange,
        Self::Allocate,
        Self::Initialize,
        Self::Take,
        Self::Replace,
        Self::DropSlot,
        Self::Release,
        Self::Size,
        Self::Align,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::WriteOwned => "pointer_initialize",
            Self::ReadOwned => "pointer_take",
            Self::Detach => "heap_detach",
            Self::Adopt => "heap_adopt",
            Self::DropInPlace => "pointer_drop",
            Self::SlotAddress => "heap_address",
            Self::SlotAddressMut => "heap_address_mut",
            Self::CheckIndex => "bounds_index",
            Self::View => "heap_view",
            Self::ViewMut => "heap_view_mut",
            Self::DropRange => "heap_drop_range",
            Self::MoveSlots => "heap_move_slots",
            Self::CheckRange => "heap_check_range",
            Self::Allocate => "heap_allocate",
            Self::Initialize => "heap_initialize",
            Self::Take => "heap_take",
            Self::Replace => "heap_replace",
            Self::DropSlot => "heap_drop",
            Self::Release => "heap_release",
            Self::Size => "layout_size",
            Self::Align => "layout_align",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::WriteOwned => "$intrinsic.pointer_initialize",
            Self::ReadOwned => "$intrinsic.pointer_take",
            Self::Detach => "$intrinsic.heap_detach",
            Self::Adopt => "$intrinsic.heap_adopt",
            Self::DropInPlace => "$intrinsic.pointer_drop",
            Self::SlotAddress => "$intrinsic.heap_address",
            Self::SlotAddressMut => "$intrinsic.heap_address_mut",
            Self::CheckIndex => "$intrinsic.bounds_index",
            Self::View => "$intrinsic.heap_view",
            Self::ViewMut => "$intrinsic.heap_view_mut",
            Self::DropRange => "$intrinsic.heap_drop_range",
            Self::MoveSlots => "$intrinsic.heap_move_slots",
            Self::CheckRange => "$intrinsic.heap_check_range",
            Self::Allocate => "$intrinsic.heap_allocate",
            Self::Initialize => "$intrinsic.heap_initialize",
            Self::Take => "$intrinsic.heap_take",
            Self::Replace => "$intrinsic.heap_replace",
            Self::DropSlot => "$intrinsic.heap_drop",
            Self::Release => "$intrinsic.heap_release",
            Self::Size => "$intrinsic.layout_size",
            Self::Align => "$intrinsic.layout_align",
        }
    }
    pub fn resolve(symbol: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.symbol() == symbol)
    }
}
