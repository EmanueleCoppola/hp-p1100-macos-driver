//! Zenographics ZjStream (ZJS) encoder for HP host-based LaserJets.
//!
//! Target device: HP LaserJet Professional P1102 (the `hp1100` PPD family).
//! Format documentation: `docs/protocol.md`.
//!
//! The encoder is pure: it writes into an in-memory buffer and never touches a
//! device, so it can be tested without a printer attached.
//!
//! Wire summary
//! -------------
//! A stream is `b"JZJZ"` followed by a flat sequence of chunks. Each chunk is a
//! 16-byte big-endian header (`size`, `type`, `items`, `reserved`, `signature`)
//! followed by `items` item records. Each item is an 8-byte big-endian header
//! (`size`, `item`, `type`, `param`) followed by its payload.

// ---------------------------------------------------------------------------
// Chunk opcodes
// ---------------------------------------------------------------------------

/// Chunk opcodes (`ZJ_TYPE`).
pub mod op {
    pub const START_DOC: u32 = 0;
    pub const END_DOC: u32 = 1;
    pub const START_PAGE: u32 = 2;
    pub const END_PAGE: u32 = 3;
    pub const JBIG_BIH: u32 = 4;
    pub const JBIG_BID: u32 = 5;
    pub const END_JBIG: u32 = 6;
    pub const SIGNATURE: u32 = 7;
    pub const RAW_IMAGE: u32 = 8;
    pub const START_PLANE: u32 = 9;
    pub const END_PLANE: u32 = 10;
}

// ---------------------------------------------------------------------------
// Item tags
// ---------------------------------------------------------------------------

/// Item tags (`ZJ_ITEM`).
pub mod item {
    pub const PAGECOUNT: u16 = 0;
    pub const DMCOLLATE: u16 = 1;
    pub const DMDUPLEX: u16 = 2;
    pub const DMPAPER: u16 = 3;
    pub const DMCOPIES: u16 = 4;
    pub const DMDEFAULTSOURCE: u16 = 5;
    pub const DMMEDIATYPE: u16 = 6;
    pub const NBIE: u16 = 7;
    pub const RESOLUTION_X: u16 = 8;
    pub const RESOLUTION_Y: u16 = 9;
    pub const OFFSET_X: u16 = 10;
    pub const OFFSET_Y: u16 = 11;
    pub const RASTER_X: u16 = 12;
    pub const RASTER_Y: u16 = 13;
    pub const QUANTITY: u16 = 15;
    pub const VIDEO_BPP: u16 = 16;
    pub const VIDEO_X: u16 = 17;
    pub const VIDEO_Y: u16 = 18;
    pub const PLANE: u16 = 20;
    pub const RET: u16 = 22;
    pub const ECONOMODE: u16 = 23;

    /// Minolta/OEM extension tags, kept because the reference encoder emits them for
    /// user and file names. Not needed for the P1102 path.
    pub const MINOLTA_USERNAME: u16 = 0x810e;
    pub const MINOLTA_FILENAME: u16 = 0x8115;
}

/// Duplex modes (`DMDUPLEX`).
pub mod duplex {
    pub const OFF: u32 = 1;
    pub const LONG_EDGE: u32 = 2;
    pub const SHORT_EDGE: u32 = 3;

    /// True for the two modes that actually duplex a page.
    pub fn is_duplexing(mode: u32) -> bool {
        mode == LONG_EDGE || mode == SHORT_EDGE
    }
}

/// Paper codes (`DMPAPER`), a small useful subset.
pub mod paper {
    pub const LETTER: u32 = 1;
    pub const LEGAL: u32 = 5;
    pub const EXECUTIVE: u32 = 7;
    pub const A3: u32 = 8;
    pub const A4: u32 = 9;
    pub const A5: u32 = 11;
    pub const B5: u32 = 13;
    pub const ENV_10: u32 = 20;
    pub const ENV_DL: u32 = 27;
    pub const ENV_C5: u32 = 28;
    pub const ENV_B5: u32 = 34;
    pub const ENV_MONARCH: u32 = 37;
    pub const POSTCARD: u32 = 43;
    pub const A6: u32 = 70;
    pub const DOUBLE_POSTCARD: u32 = 82;
    pub const CUSTOM: u32 = 256;
    pub const K16_197X273: u32 = 257;
    pub const FANFOLD_GERMAN_LEGAL: u32 = 258;
    pub const K16_184X260: u32 = 263;
    pub const K16_195X270: u32 = 264;
}

/// Input tray codes (`DMBIN`).
pub mod source {
    pub const ONLY_ONE: u32 = 1;
    pub const LOWER: u32 = 2;
    pub const MIDDLE: u32 = 3;
    pub const MANUAL: u32 = 4;
    pub const ENVELOPE: u32 = 5;
    pub const AUTO: u32 = 7;
}

/// Media type codes (`DMMEDIA`).
pub mod media {
    pub const STANDARD: u32 = 1;
    pub const TRANSPARENCY: u32 = 2;
    pub const GLOSSY: u32 = 3;
}

// ---------------------------------------------------------------------------
// Item types and fixed sizes
// ---------------------------------------------------------------------------

/// Item payload types (`ZJ_ITEM_TYPE`).
pub mod item_type {
    pub const UINT32: u8 = 1;
    pub const INT32: u8 = 2;
    pub const STRING: u8 = 3;
    pub const BYTELUT: u8 = 4;
}

pub const CHUNK_HEADER_LEN: usize = 16;
pub const ITEM_HEADER_LEN: usize = 8;
pub const ITEM_UINT32_LEN: usize = 12;
pub const CHUNK_SIGNATURE: u16 = 0x5a5a;
pub const STREAM_SIGNATURE: &[u8; 4] = b"JZJZ";
/// The JBIG Bi-level Image Header is always exactly 20 bytes.
pub const JBIG_BIH_LEN: usize = 20;
/// Cap used by the reference encoder for each JBIG data chunk.
pub const JBIG_BID_MAX: usize = 65536;

/// Zero bytes the reference implementation adds to the end of the final image-data chunk,
/// beyond rounding the chunk up to 4 bytes. It is the default of its `-X` option, and its
/// own help text describes the padding as working around what the printer expects.
pub const JBIG_BID_EXTRA_PAD: usize = 16;

/// How to fill a chunk header's `reserved` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserved {
    /// Write 0. Used by the reference encoder for item-less chunks.
    Zero,
    /// Write the total byte length of the item records that follow.
    Auto,
}

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

/// Geometry and options for one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageGeometry {
    /// True pixel width, before JBIG's 128-pixel alignment.
    pub width: u32,
    pub height: u32,
    pub resolution_x: u32,
    pub resolution_y: u32,
    /// 1 for monochrome bilevel.
    pub bits_per_pixel: u32,
    /// Emitted only when non-zero.
    pub offset_x: u32,
    pub offset_y: u32,
    pub paper: u32,
    pub media: u32,
    pub source: u32,
    pub copies: u32,
    pub duplex: u32,
    pub econo_mode: bool,
    pub ret: bool,
}

impl PageGeometry {
    /// Monochrome page at the given resolution, with the defaults the P1102 PPD uses.
    pub fn mono(width: u32, height: u32, resolution: u32) -> Self {
        Self {
            width,
            height,
            resolution_x: resolution,
            resolution_y: resolution,
            bits_per_pixel: 1,
            offset_x: 0,
            offset_y: 0,
            paper: paper::LETTER,
            media: media::STANDARD,
            source: source::ONLY_ONE,
            copies: 1,
            duplex: duplex::OFF,
            econo_mode: false,
            ret: true,
        }
    }
}

/// Job-level options for the `ZJT_START_DOC` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobOptions {
    pub duplex: u32,
    /// Emitted only when known. The reference encoder omits it on the P1102 path, which
    /// CUPS never supplies a total for.
    pub page_count: Option<u32>,
    /// Emitted only when set. The reference encoder omits it on the P1102 path.
    pub quantity: Option<u32>,
    pub collate: bool,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            duplex: duplex::OFF,
            page_count: None,
            quantity: None,
            collate: false,
        }
    }
}

/// ZjStream encoder. Writes to an in-memory buffer.
#[derive(Debug, Default)]
pub struct Encoder {
    out: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.out
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.out
    }

    /// Append raw bytes that are not part of the chunk stream, such as the PJL prologue
    /// and epilogue that wrap it.
    pub fn extend_raw(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    /// Move the bytes written so far into `out`, leaving the encoder empty.
    ///
    /// This is what lets a streaming caller hand one page at a time to the printer backend
    /// instead of accumulating a whole job in memory.
    pub fn drain_into(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.out);
    }

    /// Write the 4-byte `JZJZ` stream signature. Emitted once, before `START_DOC`.
    pub fn stream_signature(&mut self) {
        self.out.extend_from_slice(STREAM_SIGNATURE);
    }

    /// Write one chunk: 16-byte header, then the item records, then the payload.
    pub fn chunk(&mut self, ty: u32, items: &Items, payload: &[u8], reserved: Reserved) {
        let reserved = match reserved {
            Reserved::Zero => 0,
            Reserved::Auto => items.byte_len() as u16,
        };
        let size = (CHUNK_HEADER_LEN + items.byte_len() as usize + payload.len()) as u32;
        self.out.extend_from_slice(&size.to_be_bytes());
        self.out.extend_from_slice(&ty.to_be_bytes());
        self.out.extend_from_slice(&items.count().to_be_bytes());
        self.out.extend_from_slice(&reserved.to_be_bytes());
        self.out.extend_from_slice(&CHUNK_SIGNATURE.to_be_bytes());
        self.out.extend_from_slice(items.as_bytes());
        self.out.extend_from_slice(payload);
    }

    /// `ZJT_START_DOC`.
    ///
    /// The P1102 path carries `ZJI_DMCOLLATE` and, when not duplexing, `ZJI_DMDUPLEX`
    /// set to off. `ZJI_PAGECOUNT` and `ZJI_QUANTITY` are emitted only when the caller
    /// supplies them.
    pub fn doc_start(&mut self, opts: &JobOptions) {
        let mut items = Items::new();
        items.push_u32(item::DMCOLLATE, u32::from(opts.collate));
        if !duplex::is_duplexing(opts.duplex) {
            items.push_u32(item::DMDUPLEX, duplex::OFF);
        }
        if let Some(page_count) = opts.page_count {
            items.push_u32(item::PAGECOUNT, page_count);
        }
        if let Some(quantity) = opts.quantity {
            items.push_u32(item::QUANTITY, quantity);
        }
        self.chunk(op::START_DOC, &items, &[], Reserved::Auto);
    }

    /// `ZJT_END_DOC`.
    pub fn doc_end(&mut self) {
        self.chunk(op::END_DOC, &Items::new(), &[], Reserved::Auto);
    }

    /// `ZJT_START_PAGE` with the item set and ordering the P1102 path uses.
    pub fn page_start(&mut self, g: &PageGeometry) {
        let mut items = Items::new();
        if duplex::is_duplexing(g.duplex) {
            items.push_u32(item::DMDUPLEX, g.duplex);
        }
        if g.econo_mode {
            items.push_u32(item::ECONOMODE, 1);
        }
        items.push_u32(item::VIDEO_X, g.width / g.bits_per_pixel.max(1));
        items.push_u32(item::VIDEO_Y, g.height);
        items.push_u32(item::VIDEO_BPP, g.bits_per_pixel);
        items.push_u32(item::RASTER_X, g.width);
        items.push_u32(item::RASTER_Y, g.height);
        if g.offset_x != 0 {
            items.push_u32(item::OFFSET_X, g.offset_x);
        }
        if g.offset_y != 0 {
            items.push_u32(item::OFFSET_Y, g.offset_y);
        }
        // ZJI_NBIE counts bi-level image entities, i.e. bit planes: one for the mono page the
        // default path sends, two for the screened page HP's driver sends.
        items.push_u32(item::NBIE, g.bits_per_pixel.max(1));
        items.push_u32(item::RESOLUTION_X, g.resolution_x);
        items.push_u32(item::RESOLUTION_Y, g.resolution_y);
        if g.ret {
            items.push_u32(item::RET, 1);
        }
        items.push_u32(item::DMDEFAULTSOURCE, g.source);
        items.push_u32(item::DMCOPIES, g.copies);
        items.push_u32(item::DMPAPER, g.paper);
        items.push_u32(item::DMMEDIATYPE, g.media);
        self.chunk(op::START_PAGE, &items, &[], Reserved::Auto);
    }

    /// `ZJT_END_PAGE`. Carries a duplex item only while duplexing.
    pub fn page_end(&mut self, duplex_mode: u32) {
        if duplex::is_duplexing(duplex_mode) {
            let mut items = Items::new();
            items.push_u32(item::DMDUPLEX, duplex_mode);
            self.chunk(op::END_PAGE, &items, &[], Reserved::Auto);
        } else {
            self.chunk(op::END_PAGE, &Items::new(), &[], Reserved::Auto);
        }
    }

    /// The whole compressed image: `JBIG_BIH` then one or more `JBIG_BID` chunks.
    ///
    /// The last data chunk carries the reference implementation's padding: 16 zero bytes on
    /// top of rounding up to a 4-byte boundary. Earlier chunks are not padded. The padding
    /// is inert - a decoder stops once it has the announced number of lines - but matching
    /// it keeps a captured job comparable byte for byte.
    pub fn jbig_image(&mut self, bih: &[u8], data_chunks: &[&[u8]]) {
        debug_assert_eq!(bih.len(), JBIG_BIH_LEN);
        self.chunk(op::JBIG_BIH, &Items::new(), bih, Reserved::Zero);
        let last = data_chunks.len().saturating_sub(1);
        for (i, data) in data_chunks.iter().enumerate() {
            let pad = if i == last {
                JBIG_BID_EXTRA_PAD + (4 - data.len() % 4) % 4
            } else {
                0
            };
            if pad == 0 {
                self.chunk(op::JBIG_BID, &Items::new(), data, Reserved::Zero);
            } else {
                let mut padded = Vec::with_capacity(data.len() + pad);
                padded.extend_from_slice(data);
                padded.resize(data.len() + pad, 0);
                self.chunk(op::JBIG_BID, &Items::new(), &padded, Reserved::Zero);
            }
        }
    }

    /// `ZJT_END_JBIG`. Carries a duplex item only while duplexing.
    pub fn end_jbig(&mut self, duplex_mode: u32) {
        if duplex::is_duplexing(duplex_mode) {
            let mut items = Items::new();
            items.push_u32(item::DMDUPLEX, duplex_mode);
            self.chunk(op::END_JBIG, &items, &[], Reserved::Auto);
        } else {
            self.chunk(op::END_JBIG, &Items::new(), &[], Reserved::Zero);
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder, used for inspection and for golden-master comparison
// ---------------------------------------------------------------------------

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

/// Decode a complete ZjStream. Used by `p1102 zjs-dump` and by round-trip tests.
pub fn decode(stream: &[u8]) -> Result<Vec<DecodedChunk>, DecodeError> {
    if stream.len() < 4 || &stream[..4] != STREAM_SIGNATURE {
        return Err(DecodeError::BadStreamSignature);
    }
    let mut pos = 4usize;
    let mut out = Vec::new();
    while pos < stream.len() {
        if pos + CHUNK_HEADER_LEN > stream.len() {
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
        let end = pos + size as usize;
        if end > stream.len() {
            return Err(DecodeError::Truncated);
        }
        let mut ip = pos + CHUNK_HEADER_LEN;
        let mut parsed = Vec::with_capacity(items as usize);
        for _ in 0..items {
            if ip + ITEM_HEADER_LEN > end {
                return Err(DecodeError::Truncated);
            }
            let isize = be_u32(&stream[ip..])?;
            if isize < ITEM_HEADER_LEN as u32 {
                return Err(DecodeError::ChunkSizeTooSmall(isize));
            }
            let iend = ip + isize as usize;
            if iend > end {
                return Err(DecodeError::Truncated);
            }
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

// ---------------------------------------------------------------------------
// PJL framing
// ---------------------------------------------------------------------------

/// The PJL Universal Exit Language sequence.
pub const UEL: &str = "\u{1b}%-12345X";

/// PJL prologue emitted before the `JZJZ` stream signature.
///
/// Matches the reference framing for the P1102-class models. `timestamp` is passed in
/// rather than read from the clock so the output stays deterministic and testable.
/// See the note in `docs/protocol.md`: HP's macOS driver uses a slightly different PJL
/// preamble (`@PJL JOB NAME=`, `@PJL ENTER LANGUAGE=`), so exact PJL text is a
/// golden-master point of difference rather than something to match byte for byte.
/// PJL settings for a job, all of which the printer's PPD can influence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PjlSettings<'a> {
    /// `pjlDensity`, 1..5 on this printer.
    pub density: u32,
    /// `EconoMode`.
    pub econo_mode: bool,
    /// `pjlJamRecovery`: true selects `AUTO`, false `OFF`.
    pub jam_recovery_auto: bool,
    /// `YYYYMMDDHHMMSS` for the `JOBATTR` line.
    pub timestamp: &'a str,
}

impl<'a> Default for PjlSettings<'a> {
    fn default() -> Self {
        Self {
            density: 3,
            econo_mode: false,
            jam_recovery_auto: false,
            timestamp: "19700101000000",
        }
    }
}

pub fn pjl_prologue(settings: &PjlSettings<'_>) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(UEL);
    s.push_str("@PJL JOB\n");
    s.push_str(&format!(
        "@PJL SET JAMRECOVERY={}\n",
        if settings.jam_recovery_auto {
            "AUTO"
        } else {
            "OFF"
        }
    ));
    s.push_str(&format!("@PJL SET DENSITY={}\n", settings.density));
    s.push_str(&format!(
        "@PJL SET ECONOMODE={}\n",
        if settings.econo_mode { "ON" } else { "OFF" }
    ));
    s.push_str("@PJL SET RET=MEDIUM\n");
    s.push_str("@PJL INFO STATUS\n");
    s.push_str("@PJL USTATUS DEVICE = ON\n");
    s.push_str("@PJL USTATUS JOB = ON\n");
    s.push_str("@PJL USTATUS PAGE = ON\n");
    s.push_str("@PJL USTATUS TIMED = 30\n");
    s.push_str(&format!(
        "@PJL SET JOBATTR=\"JobAttr4={}\"",
        settings.timestamp
    ));
    let mut out = s.into_bytes();
    out.push(0);
    out.extend_from_slice(UEL.as_bytes());
    out
}

/// PJL epilogue emitted after `ZJT_END_DOC`.
pub fn pjl_epilogue() -> Vec<u8> {
    format!("{UEL}@PJL EOJ\n{UEL}").into_bytes()
}

/// Return the ZjStream body of a job, stripping the PJL prologue and epilogue if present.
///
/// Useful for inspection tools and for tests that want to decode a captured or generated
/// job: [`decode`] is strict about the chunk stream, so the framing has to come off first.
pub fn strip_pjl_framing(data: &[u8]) -> &[u8] {
    let start = match data
        .windows(STREAM_SIGNATURE.len())
        .position(|w| w == STREAM_SIGNATURE)
    {
        Some(p) => p,
        None => return data,
    };
    let rest = &data[start..];
    let marker = format!("{UEL}@PJL EOJ");
    match rest
        .windows(marker.len())
        .position(|w| w == marker.as_bytes())
    {
        Some(p) => &rest[..p],
        None => rest,
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;

    #[test]
    fn strip_pjl_framing_removes_prologue_and_epilogue() {
        let mut job = pjl_prologue(&PjlSettings::default());
        job.extend_from_slice(STREAM_SIGNATURE);
        let mut enc = Encoder::new();
        enc.doc_end();
        job.extend_from_slice(enc.as_bytes());
        job.extend_from_slice(&pjl_epilogue());

        let body = strip_pjl_framing(&job);
        assert_eq!(&body[..4], STREAM_SIGNATURE);
        let chunks = decode(body).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header.ty, op::END_DOC);
    }

    #[test]
    fn strip_pjl_framing_is_a_no_op_without_framing() {
        let bare = b"JZJZnothing else";
        assert_eq!(strip_pjl_framing(bare), &bare[..]);
    }
}

#[cfg(test)]
mod pjl_tests {
    use super::*;

    #[test]
    fn prologue_is_uel_delimited_and_nul_terminated() {
        let p = pjl_prologue(&PjlSettings {
            density: 3,
            econo_mode: false,
            jam_recovery_auto: false,
            timestamp: "20260915120000",
        });
        assert!(p.starts_with(UEL.as_bytes()));
        assert!(p.ends_with(UEL.as_bytes()));
        let text = String::from_utf8_lossy(&p);
        assert!(text.contains("@PJL SET DENSITY=3\n"));
        assert!(text.contains("@PJL SET ECONOMODE=OFF\n"));
        assert!(text.contains("@PJL SET JOBATTR=\"JobAttr4=20260915120000\""));
        // exactly one NUL, right before the trailing UEL
        assert_eq!(p.iter().filter(|b| **b == 0).count(), 1);
        let nul = p.iter().position(|b| *b == 0).unwrap();
        assert_eq!(&p[nul + 1..], UEL.as_bytes());
    }

    #[test]
    fn econo_mode_toggles() {
        assert!(String::from_utf8_lossy(&pjl_prologue(&PjlSettings {
            density: 1,
            econo_mode: true,
            ..PjlSettings::default()
        }))
        .contains("ECONOMODE=ON"));
    }

    #[test]
    fn jam_recovery_selects_off_or_auto() {
        let off = String::from_utf8_lossy(&pjl_prologue(&PjlSettings::default())).into_owned();
        assert!(off.contains("@PJL SET JAMRECOVERY=OFF\n"));
        let auto = String::from_utf8_lossy(&pjl_prologue(&PjlSettings {
            jam_recovery_auto: true,
            ..PjlSettings::default()
        }))
        .into_owned();
        assert!(auto.contains("@PJL SET JAMRECOVERY=AUTO\n"));
        assert!(!auto.contains("JAMRECOVERY=OFF"));
    }

    #[test]
    fn epilogue_is_uel_eoj_uel() {
        assert_eq!(
            String::from_utf8(pjl_epilogue()).unwrap(),
            format!("{UEL}@PJL EOJ\n{UEL}")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn chunk_header_layout_and_reserved() {
        let mut enc = Encoder::new();
        let mut items = Items::new();
        items.push_u32(item::DMDUPLEX, duplex::OFF);
        items.push_u32(item::PAGECOUNT, 0);
        enc.chunk(op::START_DOC, &items, &[], Reserved::Auto);
        let b = enc.as_bytes();
        // 16-byte header + 2 items of 12 bytes
        assert_eq!(b.len(), 40);
        assert_eq!(u32::from_be_bytes([b[0], b[1], b[2], b[3]]), 40);
        assert_eq!(u32::from_be_bytes([b[4], b[5], b[6], b[7]]), op::START_DOC);
        assert_eq!(u32::from_be_bytes([b[8], b[9], b[10], b[11]]), 2);
        assert_eq!(u16::from_be_bytes([b[12], b[13]]), 24); // reserved == 12 * items
        assert_eq!(u16::from_be_bytes([b[14], b[15]]), CHUNK_SIGNATURE);
    }

    #[test]
    fn doc_start_simplex_matches_reference_shape() {
        let mut enc = Encoder::new();
        enc.doc_start(&JobOptions::default());
        let chunks = decode_of(&mut enc);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header.ty, op::START_DOC);
        // The P1102 path emits exactly these two: no PAGECOUNT, no QUANTITY.
        assert_eq!(chunks[0].items.len(), 2);
        assert_eq!(chunks[0].items[0].item, item::DMCOLLATE);
        assert_eq!(chunks[0].items[1].item, item::DMDUPLEX);
        assert_eq!(value_of(&chunks[0].items, item::DMDUPLEX), duplex::OFF);
    }

    #[test]
    fn doc_start_duplex_omits_the_off_item_and_optional_items_are_opt_in() {
        let mut enc = Encoder::new();
        enc.doc_start(&JobOptions {
            duplex: duplex::LONG_EDGE,
            page_count: Some(3),
            quantity: Some(2),
            collate: true,
        });
        let chunks = decode_of(&mut enc);
        let tags: Vec<u16> = chunks[0].items.iter().map(|i| i.item).collect();
        // Duplexing pages get no DMDUPLEX in the document header; the optional items follow.
        assert_eq!(tags, vec![item::DMCOLLATE, item::PAGECOUNT, item::QUANTITY]);
        assert_eq!(value_of(&chunks[0].items, item::DMCOLLATE), 1);
        assert_eq!(value_of(&chunks[0].items, item::PAGECOUNT), 3);
        assert_eq!(value_of(&chunks[0].items, item::QUANTITY), 2);
    }

    #[test]
    fn page_start_has_no_plane_items_and_uses_true_width() {
        let mut enc = Encoder::new();
        let mut g = PageGeometry::mono(4900, 6400, 600);
        g.ret = false;
        enc.page_start(&g);
        let chunks = decode_of(&mut enc);
        let items = &chunks[0].items;
        let tags: Vec<u16> = items.iter().map(|i| i.item).collect();
        assert_eq!(
            tags,
            vec![
                item::VIDEO_X,
                item::VIDEO_Y,
                item::VIDEO_BPP,
                item::RASTER_X,
                item::RASTER_Y,
                item::NBIE,
                item::RESOLUTION_X,
                item::RESOLUTION_Y,
                item::DMDEFAULTSOURCE,
                item::DMCOPIES,
                item::DMPAPER,
                item::DMMEDIATYPE,
            ]
        );
        assert!(!tags.contains(&item::PLANE));
        assert!(!tags.contains(&item::ECONOMODE));
        assert_eq!(value_of(items, item::RASTER_X), 4900);
        assert_eq!(value_of(items, item::RASTER_Y), 6400);
        assert_eq!(value_of(items, item::VIDEO_X), 4900);
        assert_eq!(value_of(items, item::VIDEO_BPP), 1);
        assert_eq!(value_of(items, item::RESOLUTION_X), 600);
    }

    #[test]
    fn page_start_emits_optional_items_only_when_set() {
        let mut g = PageGeometry::mono(100, 100, 600);
        g.offset_x = 5;
        g.offset_y = 7;
        g.econo_mode = true;
        let mut enc = Encoder::new();
        enc.page_start(&g);
        let chunks = decode_of(&mut enc);
        let tags: Vec<u16> = chunks[0].items.iter().map(|i| i.item).collect();
        assert!(tags.contains(&item::ECONOMODE));
        assert!(tags.contains(&item::OFFSET_X));
        assert!(tags.contains(&item::OFFSET_Y));
        assert_eq!(value_of(&chunks[0].items, item::OFFSET_X), 5);
        assert_eq!(value_of(&chunks[0].items, item::OFFSET_Y), 7);
        // RESOLUTION_X must be followed by RESOLUTION_Y then RET
        let rx = tags.iter().position(|t| *t == item::RESOLUTION_X).unwrap();
        assert_eq!(tags[rx + 1], item::RESOLUTION_Y);
        assert_eq!(tags[rx + 2], item::RET);
    }

    #[test]
    fn duplex_adds_items_to_page_end_and_end_jbig() {
        let mut enc = Encoder::new();
        enc.page_end(duplex::LONG_EDGE);
        enc.end_jbig(duplex::LONG_EDGE);
        let chunks = decode_of(&mut enc);
        assert_eq!(chunks[0].header.ty, op::END_PAGE);
        assert_eq!(chunks[0].items.len(), 1);
        assert_eq!(
            value_of(&chunks[0].items, item::DMDUPLEX),
            duplex::LONG_EDGE
        );
        assert_eq!(chunks[1].header.ty, op::END_JBIG);
        assert_eq!(chunks[1].items.len(), 1);

        let mut enc = Encoder::new();
        enc.page_end(duplex::OFF);
        let chunks = decode_of(&mut enc);
        assert_eq!(chunks[0].items.len(), 0);
        assert_eq!(chunks[0].header.reserved, 0);
    }

    #[test]
    fn jbig_image_pads_only_the_last_chunk() {
        let bih = [0u8; JBIG_BIH_LEN];
        let a = [1u8, 2, 3]; // not last: never padded
        let b = [9u8; 7]; // last: 7 + extra 16 + 1 to reach 4 bytes
        let mut enc = Encoder::new();
        enc.jbig_image(&bih, &[&a, &b]);
        let chunks = decode_of(&mut enc);
        assert_eq!(chunks[0].header.ty, op::JBIG_BIH);
        assert_eq!(chunks[0].header.items, 0);
        assert_eq!(
            chunks[0].header.size as usize,
            CHUNK_HEADER_LEN + JBIG_BIH_LEN
        );
        assert_eq!(chunks[0].payload.len(), JBIG_BIH_LEN);
        assert_eq!(chunks[1].header.ty, op::JBIG_BID);
        assert_eq!(
            chunks[1].payload.len(),
            3,
            "non-final chunk must not be padded"
        );
        assert_eq!(
            chunks[2].payload.len(),
            7 + JBIG_BID_EXTRA_PAD + 1,
            "the final chunk carries the reference encoder's extra padding"
        );
        assert_eq!(&chunks[2].payload[7..], &[0u8; JBIG_BID_EXTRA_PAD + 1]);
    }

    #[test]
    fn stream_signature_precedes_start_doc() {
        let mut enc = Encoder::new();
        enc.stream_signature();
        enc.doc_start(&JobOptions::default());
        assert_eq!(&enc.as_bytes()[..4], STREAM_SIGNATURE);
        let chunks = decode(enc.as_bytes()).unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn decode_rejects_a_missing_stream_signature() {
        assert_eq!(
            decode(b"XXXX").unwrap_err(),
            DecodeError::BadStreamSignature
        );
        assert_eq!(decode(b"").unwrap_err(), DecodeError::BadStreamSignature);
    }

    /// These are what `p1102 zjs-dump` prints when it is pointed at something that is not a
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
    /// Byte-level golden vector for a full `START_PAGE`, written out explicitly from the
    /// format description rather than by reusing the encoder's own helpers. This is the
    /// test that would catch a systematic mistake shared by the encoder and the decoder.
    #[test]
    fn start_page_matches_a_hand_written_golden_vector() {
        let g = PageGeometry::mono(8, 8, 600);
        let mut enc = Encoder::new();
        enc.page_start(&g);

        // Item order and values for a plain monochrome page, read off docs/protocol.md.
        let expected_items: &[(u16, u32)] = &[
            (item::VIDEO_X, 8),
            (item::VIDEO_Y, 8),
            (item::VIDEO_BPP, 1),
            (item::RASTER_X, 8),
            (item::RASTER_Y, 8),
            (item::NBIE, 1),
            (item::RESOLUTION_X, 600),
            (item::RESOLUTION_Y, 600),
            (item::RET, 1),
            (item::DMDEFAULTSOURCE, source::ONLY_ONE),
            (item::DMCOPIES, 1),
            (item::DMPAPER, paper::LETTER),
            (item::DMMEDIATYPE, media::STANDARD),
        ];

        let mut want = Vec::new();
        let item_bytes = 12 * expected_items.len() as u32;
        want.extend_from_slice(&(CHUNK_HEADER_LEN as u32 + item_bytes).to_be_bytes());
        want.extend_from_slice(&op::START_PAGE.to_be_bytes());
        want.extend_from_slice(&(expected_items.len() as u32).to_be_bytes());
        want.extend_from_slice(&(item_bytes as u16).to_be_bytes());
        want.extend_from_slice(&0x5a5au16.to_be_bytes());
        for (tag, value) in expected_items {
            want.extend_from_slice(&(ITEM_UINT32_LEN as u32).to_be_bytes());
            want.extend_from_slice(&tag.to_be_bytes());
            want.push(1); // ZJIT_UINT32
            want.push(0); // param
            want.extend_from_slice(&value.to_be_bytes());
        }

        assert_eq!(enc.as_bytes(), want.as_slice());
        // 13 items of 12 bytes -> the reserved field is 0x9c, which is the same constant
        // the reference implementation uses for this model family.
        assert_eq!(item_bytes, 0x9c);
    }

    #[test]
    fn full_mono_job_round_trips() {
        let g = PageGeometry::mono(4900, 6400, 600);
        let bih = {
            let mut v = vec![0u8; JBIG_BIH_LEN];
            v[4..8].copy_from_slice(&4992u32.to_be_bytes()); // 128-aligned
            v[8..12].copy_from_slice(&6400u32.to_be_bytes());
            v
        };
        let mut enc = Encoder::new();
        enc.stream_signature();
        enc.doc_start(&JobOptions::default());
        enc.page_start(&g);
        enc.jbig_image(&bih, &[&[0xabu8; 100]]);
        enc.end_jbig(duplex::OFF);
        enc.page_end(duplex::OFF);
        enc.doc_end();

        let chunks = decode(enc.as_bytes()).unwrap();
        let ops: Vec<u32> = chunks.iter().map(|c| c.header.ty).collect();
        assert_eq!(
            ops,
            vec![
                op::START_DOC,
                op::START_PAGE,
                op::JBIG_BIH,
                op::JBIG_BID,
                op::END_JBIG,
                op::END_PAGE,
                op::END_DOC,
            ]
        );
        // Every chunk's size must account for header + items + payload exactly.
        for c in &chunks {
            let item_bytes: usize = c.items.iter().map(|i| i.size as usize).sum();
            assert_eq!(
                c.header.size as usize,
                CHUNK_HEADER_LEN + item_bytes + c.payload.len()
            );
        }
        // reserved mirrors the item byte count wherever items are present.
        for c in &chunks {
            if c.header.items > 0 {
                assert_eq!(c.header.reserved as usize, 12 * c.header.items as usize);
            }
        }
    }

    fn decode_of(enc: &mut Encoder) -> Vec<DecodedChunk> {
        let mut buf = vec![0u8; 0];
        buf.extend_from_slice(STREAM_SIGNATURE);
        buf.extend_from_slice(enc.as_bytes());
        decode(&buf).unwrap()
    }

    fn value_of(items: &[DecodedItem], tag: u16) -> u32 {
        let it = items.iter().find(|i| i.item == tag).expect("item present");
        u32::from_be_bytes([it.payload[0], it.payload[1], it.payload[2], it.payload[3]])
    }
}
