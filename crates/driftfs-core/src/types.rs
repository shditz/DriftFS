use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub String);

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub String);

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MountId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

impl ByteRange {
    pub fn new(offset: u64, length: u64) -> Self {
        Self { offset, length }
    }

    pub fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }

    pub fn end_inclusive(&self) -> u64 {
        if self.length == 0 {
            self.offset
        } else {
            self.end().saturating_sub(1)
        }
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.offset < other.end() && other.offset < self.end()
    }

    pub fn contains_range(&self, other: &Self) -> bool {
        self.offset <= other.offset && other.end() <= self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_range_end() {
        let r = ByteRange::new(100, 50);
        assert_eq!(r.end(), 150);
    }

    #[test]
    fn byte_range_overlap_detection() {
        let a = ByteRange::new(0, 100);
        let b = ByteRange::new(50, 100);
        let c = ByteRange::new(100, 50);

        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c), "adjacent ranges do not overlap");
    }

    #[test]
    fn byte_range_containment() {
        let outer = ByteRange::new(0, 200);
        let inner = ByteRange::new(50, 100);
        let partial = ByteRange::new(150, 100);

        assert!(outer.contains_range(&inner));
        assert!(!outer.contains_range(&partial));
    }

    #[test]
    fn file_id_display() {
        let id = FileId("abc123".into());
        assert_eq!(id.to_string(), "abc123");
    }

    #[test]
    fn byte_range_end_saturates() {
        let r = ByteRange::new(u64::MAX - 10, 100);
        assert_eq!(r.end(), u64::MAX);
    }
}
