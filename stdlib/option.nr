pub enum Option<T> {
    Some(T),
    None,
}

impl<T> Option<T> {
    /// Returns true when `value` contains a payload.
    pub is_some(self) bool {
        return match self {
            Some(_) => true,
            None => false,
        };
    }

    /// Extracts an Option payload or returns the provided fallback.
    pub unwrap_or(self, fallback T) T {
        return match self {
            Some(item) => item,
            None => fallback,
        };
    }

    /// Applies `mapper` to an Option payload.
    pub map<U>(self, mapper (T) => U) Option<U> {
        return match self {
            Some(item) => Option.Some(mapper(item)),
            None => Option.None,
        };
    }
}
