use nether_semantics::{
    AggregateLayout, EnumLayout, IntegerType, Layout, LayoutError, MAX_OBJECT_SIZE,
};

#[test]
fn primitives_use_target_layout_instead_of_host_layout() {
    for (ty, bytes) in [
        (IntegerType::I8, 1),
        (IntegerType::U8, 1),
        (IntegerType::I16, 2),
        (IntegerType::U16, 2),
        (IntegerType::I32, 4),
        (IntegerType::U32, 4),
        (IntegerType::I64, 8),
        (IntegerType::U64, 8),
        (IntegerType::I128, 16),
        (IntegerType::U128, 16),
        (IntegerType::Isize, 8),
        (IntegerType::Usize, 8),
    ] {
        assert_eq!(Layout::integer(ty), Layout::new(bytes, bytes).unwrap());
    }
    assert_eq!((Layout::UNIT.size(), Layout::UNIT.align()), (0, 1));
    assert_eq!((Layout::STR.size(), Layout::STR.align()), (16, 8));
}

#[test]
fn fields_keep_declaration_order_and_receive_tail_padding() {
    let aggregate =
        AggregateLayout::new(&[Layout::BOOL, Layout::POINTER, Layout::CHAR, Layout::BOOL]).unwrap();
    assert_eq!(aggregate.field_offsets, [0, 8, 16, 20]);
    assert_eq!(aggregate.layout, Layout::new(24, 8).unwrap());
    let nested = AggregateLayout::new(&[Layout::BOOL, aggregate.layout, Layout::BOOL]).unwrap();
    assert_eq!(nested.field_offsets, [0, 8, 32]);
    assert_eq!(nested.layout.size(), 40);
    assert_eq!(nested.layout.array(3).unwrap().size(), 120);
}

#[test]
fn zero_sized_and_overaligned_fields_keep_alignment() {
    let marker = Layout::new(0, 64).unwrap();
    let aggregate = AggregateLayout::new(&[Layout::BOOL, marker, Layout::BOOL]).unwrap();
    assert_eq!(aggregate.field_offsets, [0, 64, 64]);
    assert_eq!(aggregate.layout, Layout::new(128, 64).unwrap());
    assert_eq!(marker.array(u64::MAX).unwrap(), marker);
    assert_eq!(Layout::new(64, 64).unwrap().array(0).unwrap(), marker);
    assert_eq!(AggregateLayout::new(&[]).unwrap().layout, Layout::UNIT);
}

#[test]
fn enums_use_a_separate_tag_and_aligned_payload() {
    let payload = AggregateLayout::new(&[Layout::BOOL, Layout::F64]).unwrap();
    let enumeration = EnumLayout::new(&[Layout::UNIT, payload.layout, Layout::CHAR]).unwrap();
    assert_eq!(enumeration.tag, Some(Layout::integer(IntegerType::U32)));
    assert_eq!(enumeration.payload_offset, 8);
    assert_eq!(enumeration.layout, Layout::new(24, 8).unwrap());
    let zst = EnumLayout::new(&[Layout::UNIT, Layout::UNIT]).unwrap();
    assert_eq!(zst.layout, Layout::new(4, 4).unwrap());
    let uninhabited = EnumLayout::new(&[]).unwrap();
    assert_eq!(uninhabited.layout, Layout::UNIT);
    assert_eq!(uninhabited.tag, None);
}

#[test]
fn invalid_sizes_and_overflows_are_errors_in_both_rust_profiles() {
    assert_eq!(Layout::new(0, 0), Err(LayoutError::InvalidAlignment));
    assert_eq!(Layout::new(6, 3), Err(LayoutError::InvalidAlignment));
    assert_eq!(Layout::new(3, 4), Err(LayoutError::UnpaddedSize));
    assert_eq!(Layout::new(u64::MAX, 1), Err(LayoutError::ObjectTooLarge));
    assert_eq!(Layout::new(0, 1 << 63), Err(LayoutError::ObjectTooLarge));
    let large = Layout::new(MAX_OBJECT_SIZE, 1).unwrap();
    assert_eq!(large.array(2), Err(LayoutError::ObjectTooLarge));
    assert_eq!(
        Layout::POINTER.array(u64::MAX),
        Err(LayoutError::ObjectTooLarge)
    );
    assert_eq!(
        AggregateLayout::new(&[large, Layout::UNIT, Layout::BOOL]),
        Err(LayoutError::ObjectTooLarge)
    );
    assert_eq!(
        AggregateLayout::new(&[large, Layout::POINTER]),
        Err(LayoutError::ObjectTooLarge)
    );
    assert_eq!(EnumLayout::new(&[large]), Err(LayoutError::ObjectTooLarge));
}

#[test]
fn enumerated_small_aggregates_have_aligned_nonoverlapping_storage() {
    let fields = [
        Layout::UNIT,
        Layout::BOOL,
        Layout::CHAR,
        Layout::POINTER,
        Layout::new(16, 16).unwrap(),
        Layout::new(0, 32).unwrap(),
    ];
    for first in fields {
        for second in fields {
            for third in fields {
                let input = [first, second, third];
                let output = AggregateLayout::new(&input).unwrap();
                let mut end = 0;
                for (field, offset) in input.iter().zip(output.field_offsets) {
                    assert!(offset >= end);
                    assert_eq!(offset % field.align(), 0);
                    // No unnecessary padding beyond the next required alignment.
                    assert!(offset - end < field.align());
                    end = offset + field.size();
                }
                assert!(output.layout.size() >= end);
                assert_eq!(output.layout.size() % output.layout.align(), 0);
                assert!(output.layout.size() - end < output.layout.align());
            }
        }
    }
}
