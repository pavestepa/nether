enum Result<T, E> {
    Ok(T),
    Error(E),
}

impl<T, E> Result<T, E> {
    /// Returns true when `self` is the successful Result variant.
    is_ok(self) bool {
        return match self {
            Ok(_) => true,
            Error(_) => false,
        };
    }

    /// Extracts a successful Result payload or returns a fallback.
    unwrap_or(self, fallback T) T {
        return match self {
            Ok(item) => item,
            Error(_) => fallback,
        };
    }

    /// Maps only the successful side of a Result.
    map<U>(self, mapper (T) => U) Result<U, E> {
        return match self {
            Ok(item) => Result.Ok(mapper(item)),
            Error(error) => Result.Error(error),
        };
    }
}
