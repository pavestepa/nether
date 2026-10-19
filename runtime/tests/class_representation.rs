//! Executable representation contract; source class lowering is still pending.
struct AData {
    value: i32,
}
struct BData {
    base: AData,
    extra: i32,
}
trait A {
    fn value(&self) -> i32;
    fn increment(&mut self);
}
impl A for AData {
    fn value(&self) -> i32 {
        self.value
    }
    fn increment(&mut self) {
        self.value += 1;
    }
}
impl A for BData {
    fn value(&self) -> i32 {
        self.base.value + self.extra
    }
    fn increment(&mut self) {
        self.base.value += 1;
    }
}
fn statically_specialized<T: A>(value: &mut T) -> i32 {
    value.increment();
    value.value()
}
#[test]
fn static_and_dynamic_upcasts_preserve_allocation_identity() {
    let mut concrete = Box::new(BData {
        base: AData { value: 3 },
        extra: 7,
    });
    let identity = (&*concrete as *const BData).cast::<()>();
    assert_eq!(statically_specialized(&mut *concrete), 11);
    let mut erased: Box<dyn A> = concrete;
    assert_eq!((&*erased as *const dyn A).cast::<()>(), identity);
    erased.increment();
    assert_eq!(erased.value(), 12);
    let mixed: Vec<Box<dyn A>> = vec![Box::new(AData { value: 2 }), erased];
    assert_eq!(mixed.iter().map(|v| v.value()).sum::<i32>(), 14);
    assert_eq!((&*mixed[1] as *const dyn A).cast::<()>(), identity);
}
