//! Writing the chunk stream: headers, item blocks and the payload each chunk carries.

use crate::items::Items;
use crate::job::JobOptions;
use crate::page::PageGeometry;
use crate::wire::{
    Reserved, CHUNK_HEADER_LEN, CHUNK_SIGNATURE, JBIG_BID_EXTRA_PAD, JBIG_BIH_LEN, STREAM_SIGNATURE,
};
use crate::{duplex, item, op};

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

    /// Discard the encoded bytes while retaining the buffer allocation.
    ///
    /// This is useful for a streaming caller that writes `as_bytes()` directly and
    /// then starts the next chunk or page without a second pending buffer.
    pub fn clear(&mut self) {
        self.out.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{decode, DecodedChunk, DecodedItem};
    use crate::wire::ITEM_UINT32_LEN;
    use crate::{media, paper, source};
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
        // The reference emits exactly these two for this model: no PAGECOUNT, no QUANTITY.
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

    /// A two-bit page: the same items, with the depth carried through them the way that path
    /// needs it. VIDEO_X is the width in pixels over the bits per pixel, NBIE counts the two
    /// bi-level image entities, and RASTER_X stays the page's true width in pixels.
    #[test]
    fn page_start_carries_a_two_bit_depth() {
        let mut enc = Encoder::new();
        let mut g = PageGeometry::mono(4900, 6400, 600);
        g.bits_per_pixel = 2;
        g.ret = false;
        enc.page_start(&g);
        let chunks = decode_of(&mut enc);
        let items = &chunks[0].items;
        assert_eq!(value_of(items, item::VIDEO_BPP), 2);
        assert_eq!(value_of(items, item::VIDEO_X), 2450);
        assert_eq!(value_of(items, item::NBIE), 2);
        assert_eq!(value_of(items, item::RASTER_X), 4900);
        assert_eq!(value_of(items, item::RASTER_Y), 6400);
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
    fn clear_reuses_the_encoder_buffer() {
        let mut enc = Encoder::new();
        enc.extend_raw(&[0u8; 128]);
        let capacity = enc.out.capacity();

        enc.clear();

        assert!(enc.as_bytes().is_empty());
        assert_eq!(enc.out.capacity(), capacity);
        enc.extend_raw(&[1u8; 128]);
        assert_eq!(enc.as_bytes(), &[1u8; 128]);
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
