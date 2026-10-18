//! The item records that follow a chunk header.
//!
//! An item is an 8-byte header (`size`, `item`, `type`, `param`) followed by its
//! payload; a chunk's items are written as one contiguous block. `Items` builds that
//! block so a caller can hand it to `Encoder::chunk`.

use crate::item_type;
use crate::wire::{ITEM_HEADER_LEN, ITEM_UINT32_LEN};

/// A set of item records belonging to one chunk.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Items {
    bytes: Vec<u8>,
    count: u32,
}

impl Items {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a `ZJIT_UINT32` item.
    pub fn push_u32(&mut self, tag: u16, value: u32) -> &mut Self {
        self.bytes
            .extend_from_slice(&(ITEM_UINT32_LEN as u32).to_be_bytes());
        self.bytes.extend_from_slice(&tag.to_be_bytes());
        self.bytes.push(item_type::UINT32);
        self.bytes.push(0);
        self.bytes.extend_from_slice(&value.to_be_bytes());
        self.count += 1;
        self
    }

    /// Append a `ZJIT_STRING` item. The payload is NUL-terminated and padded to a
    /// 4-byte multiple, and the padding is included in the item size.
    pub fn push_str(&mut self, tag: u16, value: &str) -> &mut Self {
        let raw = value.as_bytes();
        // NUL-terminated, then rounded up to a whole number of 4-byte words
        let padded = (raw.len() + 1).div_ceil(4) * 4;
        self.bytes
            .extend_from_slice(&((ITEM_HEADER_LEN + padded) as u32).to_be_bytes());
        self.bytes.extend_from_slice(&tag.to_be_bytes());
        self.bytes.push(item_type::STRING);
        self.bytes.push(0);
        self.bytes.extend_from_slice(raw);
        self.bytes.resize(self.bytes.len() + padded - raw.len(), 0);
        self.count += 1;
        self
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn byte_len(&self) -> u32 {
        self.bytes.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item;

    #[test]
    fn uint32_item_is_twelve_bytes_big_endian() {
        let mut items = Items::new();
        items.push_u32(item::DMCOLLATE, 0);
        assert_eq!(items.count(), 1);
        assert_eq!(items.byte_len(), 12);
        assert_eq!(
            items.as_bytes(),
            &[0x00, 0x00, 0x00, 0x0c, 0x00, 0x01, 0x01, 0x00, 0, 0, 0, 0]
        );
    }

    #[test]
    fn string_item_is_padded_to_four_bytes() {
        let mut items = Items::new();
        // "TEST" + NUL = 5 bytes -> padded to 8 -> total size 8 + 8 = 16
        items.push_str(item::MINOLTA_USERNAME, "TEST");
        assert_eq!(items.byte_len(), 16);
        assert_eq!(&items.as_bytes()[..4], &[0, 0, 0, 16]);
        assert_eq!(items.as_bytes()[8], b'T');
        assert_eq!(items.as_bytes()[12], b'\0');
    }

    #[test]
    fn string_item_with_exact_four_byte_alignment() {
        let mut items = Items::new();
        // "ABC" + NUL = 4 bytes, already aligned -> total 12
        items.push_str(item::MINOLTA_USERNAME, "ABC");
        assert_eq!(items.byte_len(), 12);
    }
}
