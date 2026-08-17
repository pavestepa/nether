pub struct Array<T>;

impl<T> Array<T> {
    map(mut self, iter (T) => ()) {
        let mut i usize = 0;
        while i < self.len() {
            iter(self[i]);
            i = i + 1;
        }
    }
}
