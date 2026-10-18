//! Reading a chunk stream back: used for inspection and for golden-master comparison.

use crate::op;
use crate::wire::{CHUNK_HEADER_LEN, CHUNK_SIGNATURE, ITEM_HEADER_LEN, STREAM_SIGNATURE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkHeader {
    pub size: u32,
    pub ty: u32,
    pub items: u32,
    pub reserved: u16,
    pub signature: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedItem {
    pub size: u32,
    pub item: u16,
    pub ty: u8,
    pub param: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedChunk {
    pub header: ChunkHeader,
    pub items: Vec<DecodedItem>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    BadStreamSignature,
    Truncated,
    BadChunkSignature(u16),
    ChunkSizeTooSmall(u32),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::BadStreamSignature => {
                write!(f, "not a ZjStream: it has to begin with the JZJZ signature")
            }
            DecodeError::Truncated => {
                write!(f, "the stream ends in the middle of a chunk header")
            }
            DecodeError::BadChunkSignature(found) => write!(
                f,
                "chunk signature {found:#06x} where {:#06x} was expected",
                CHUNK_SIGNATURE
            ),
            DecodeError::ChunkSizeTooSmall(size) => write!(
                f,
                "a chunk claims {size} bytes, less than its own {CHUNK_HEADER_LEN}-byte header"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode a complete ZjStream. Used by `p1100 zjs-dump` and by round-trip tests.
pub fn decode(stream: &[u8]) -> Result<Vec<DecodedChunk>, DecodeError> {
    if stream.len() < 4 || &stream[..4] != STREAM_SIGNATURE {
        return Err(DecodeError::BadStreamSignature);
    }
    let mut pos = 4usize;
    let mut out = Vec::new();
    while pos < stream.len() {
        if stream.len() - pos < CHUNK_HEADER_LEN {
            return Err(DecodeError::Truncated);
        }
        let size = be_u32(&stream[pos..])?;
        let ty = be_u32(&stream[pos + 4..])?;
        let items = be_u32(&stream[pos + 8..])?;
        let reserved = be_u16(&stream[pos + 12..])?;
        let signature = be_u16(&stream[pos + 14..])?;
        if signature != CHUNK_SIGNATURE {
            return Err(DecodeError::BadChunkSignature(signature));
        }
        if (size as usize) < CHUNK_HEADER_LEN {
            return Err(DecodeError::ChunkSizeTooSmall(size));
        }
        if size as usize > stream.len() - pos {
            return Err(DecodeError::Truncated);
        }
        let end = pos + size as usize;
        let mut ip = pos + CHUNK_HEADER_LEN;
        // Reject impossible counts before parsing and grow only as items are validated.
        // A tiny malformed chunk must not trigger an allocation based on `items`.
        let item_capacity = (end - (pos + CHUNK_HEADER_LEN)) / ITEM_HEADER_LEN;
        if items as usize > item_capacity {
            return Err(DecodeError::Truncated);
        }
        let mut parsed = Vec::new();
        for _ in 0..items {
            if end - ip < ITEM_HEADER_LEN {
                return Err(DecodeError::Truncated);
            }
            let isize = be_u32(&stream[ip..])?;
            if isize < ITEM_HEADER_LEN as u32 {
                return Err(DecodeError::ChunkSizeTooSmall(isize));
            }
            if isize as usize > end - ip {
                return Err(DecodeError::Truncated);
            }
            let iend = ip + isize as usize;
            parsed.push(DecodedItem {
                size: isize,
                item: be_u16(&stream[ip + 4..])?,
                ty: stream[ip + 6],
                param: stream[ip + 7],
                payload: stream[ip + ITEM_HEADER_LEN..iend].to_vec(),
            });
            ip = iend;
        }
        out.push(DecodedChunk {
            header: ChunkHeader {
                size,
                ty,
                items,
                reserved,
                signature,
            },
            items: parsed,
            payload: stream[ip..end].to_vec(),
        });
        pos = end;
    }
    Ok(out)
}

fn be_u32(b: &[u8]) -> Result<u32, DecodeError> {
    if b.len() < 4 {
        return Err(DecodeError::Truncated);
    }
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be_u16(b: &[u8]) -> Result<u16, DecodeError> {
    if b.len() < 2 {
        return Err(DecodeError::Truncated);
    }
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

/// Human-readable name for a chunk opcode.
pub fn op_name(ty: u32) -> &'static str {
    match ty {
        op::START_DOC => "START_DOC",
        op::END_DOC => "END_DOC",
        op::START_PAGE => "START_PAGE",
        op::END_PAGE => "END_PAGE",
        op::JBIG_BIH => "JBIG_BIH",
        op::JBIG_BID => "JBIG_BID",
        op::END_JBIG => "END_JBIG",
        op::SIGNATURE => "SIGNATURE",
        op::RAW_IMAGE => "RAW_IMAGE",
        op::START_PLANE => "START_PLANE",
        op::END_PLANE => "END_PLANE",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Encoder, JobOptions};
    #[test]
    fn decode_rejects_a_missing_stream_signature() {
        assert_eq!(
            decode(b"XXXX").unwrap_err(),
            DecodeError::BadStreamSignature
        );
        assert_eq!(decode(b"").unwrap_err(), DecodeError::BadStreamSignature);
    }

    #[test]
    fn decode_does_not_reserve_from_an_untrusted_item_count() {
        let mut malformed = Vec::from(STREAM_SIGNATURE.as_slice());
        malformed.extend_from_slice(&16u32.to_be_bytes());
        malformed.extend_from_slice(&op::START_DOC.to_be_bytes());
        malformed.extend_from_slice(&u32::MAX.to_be_bytes());
        malformed.extend_from_slice(&0u16.to_be_bytes());
        malformed.extend_from_slice(&CHUNK_SIGNATURE.to_be_bytes());

        assert_eq!(decode(&malformed).unwrap_err(), DecodeError::Truncated);
    }

    #[test]
    fn maximum_lengths_are_rejected_without_wrapping_on_32_bit_targets() {
        let mut enc = Encoder::new();
        enc.stream_signature();
        enc.doc_start(&JobOptions::default());
        let bytes = enc.into_bytes();
        let mut bad_chunk = bytes.clone();
        bad_chunk[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode(&bad_chunk).unwrap_err(), DecodeError::Truncated);
        let mut bad_item = bytes;
        bad_item[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode(&bad_item).unwrap_err(), DecodeError::Truncated);
    }

    /// These are what `p1100 zjs-dump` prints when it is pointed at something that is not a
    /// job - a CUPS raster, say - so the wording is part of the tool's interface.
    #[test]
    fn decode_errors_say_what_is_wrong() {
        assert_eq!(
            DecodeError::BadStreamSignature.to_string(),
            "not a ZjStream: it has to begin with the JZJZ signature"
        );
        assert_eq!(
            DecodeError::BadChunkSignature(0x1234).to_string(),
            "chunk signature 0x1234 where 0x5a5a was expected"
        );
        assert_eq!(
            DecodeError::Truncated.to_string(),
            "the stream ends in the middle of a chunk header"
        );
        assert_eq!(
            DecodeError::ChunkSizeTooSmall(4).to_string(),
            "a chunk claims 4 bytes, less than its own 16-byte header"
        );
    }
}
