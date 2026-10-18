//! Boundary allocations have explicit limits; decoders must also limit before allocating.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitExceeded;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Text<const BYTES: usize>(String);

impl<const BYTES: usize> Text<BYTES> {
    pub fn new(value: &str) -> Result<Self, LimitExceeded> {
        if value.len() > BYTES {
            return Err(LimitExceeded);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<T, const ITEMS: usize>(Vec<T>);

impl<T, const ITEMS: usize> Page<T, ITEMS> {
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Incremental insertion prevents an unbounded iterator from being collected first.
    pub fn try_push(&mut self, item: T) -> Result<(), LimitExceeded> {
        if self.0.len() >= ITEMS {
            return Err(LimitExceeded);
        }
        self.0.push(item);
        Ok(())
    }

    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}
