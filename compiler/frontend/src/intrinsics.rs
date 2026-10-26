//! Compiler-owned module. Private symbols cannot be spelled by source programs.
pub const MODULE: &str = "core/intrinsic";
pub const SOURCE: &str = include_str!("intrinsic.nr");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
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
    pub const ALL: [Self; 8] = [
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
