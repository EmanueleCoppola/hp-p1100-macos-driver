//! Working with ZjStream bytes directly: reading an encoded job back, and framing an image
//! that is already encoded.

use std::fs;

use p1100_protocol::{
    decode, op_name, pjl_epilogue, pjl_prologue, strip_pjl_framing, Encoder, JobOptions,
    PageGeometry, PjlSettings,
};

use crate::read_file;

pub(crate) fn zjs_dump(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("zjs-dump needs a file")?;
    let raw = read_file(path)?;
    let framed = raw.windows(4).any(|w| w == b"@PJL");
    let data = strip_pjl_framing(&raw);
    if framed {
        println!("PJL framing present; decoding the ZjStream body only");
    }
    let chunks = decode(data).map_err(|e| format!("{e}"))?;
    let mut total_items = 0usize;
    let mut payload = 0usize;
    for (i, c) in chunks.iter().enumerate() {
        println!(
            "[{i:03}] {:<11} size={:<7} items={:<3} reserved={:#06x} sig={:#06x}",
            op_name(c.header.ty),
            c.header.size,
            c.header.items,
            c.header.reserved,
            c.header.signature
        );
        for it in &c.items {
            let value = if it.ty == p1100_protocol::item_type::UINT32 && it.payload.len() == 4 {
                format!(
                    "u32={}",
                    u32::from_be_bytes([
                        it.payload[0],
                        it.payload[1],
                        it.payload[2],
                        it.payload[3]
                    ])
                )
            } else if it.ty == p1100_protocol::item_type::STRING {
                let end = it
                    .payload
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(it.payload.len());
                format!("str={:?}", String::from_utf8_lossy(&it.payload[..end]))
            } else {
                format!("{} bytes", it.payload.len())
            };
            println!(
                "       item tag=0x{:04x} type={} size={} param={} {}",
                it.item, it.ty, it.size, it.param, value
            );
            total_items += 1;
        }
        if !c.payload.is_empty() {
            println!("       payload {} bytes", c.payload.len());
        }
        payload += c.payload.len();
    }
    println!(
        "chunks: {}  items: {}  payload bytes: {}",
        chunks.len(),
        total_items,
        payload
    );
    Ok(())
}
pub(crate) fn wrap(args: &[String]) -> Result<(), String> {
    let mut bih: Option<String> = None;
    let mut bids: Vec<String> = Vec::new();
    let mut out: Option<String> = None;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut resolution = 600u32;
    let mut paper = p1100_protocol::paper::LETTER;
    let mut bits = 1u32;
    let mut pjl = true;

    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<&String, String> {
            args.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--bih" => {
                bih = Some(need(i)?.clone());
                i += 2;
            }
            "--bid" => {
                bids.push(need(i)?.clone());
                i += 2;
            }
            "--width" => {
                width = need(i)?.parse().map_err(|_| "bad --width")?;
                i += 2;
            }
            "--height" => {
                height = need(i)?.parse().map_err(|_| "bad --height")?;
                i += 2;
            }
            "--resolution" => {
                resolution = need(i)?.parse().map_err(|_| "bad --resolution")?;
                i += 2;
            }
            "--paper-code" => {
                paper = need(i)?.parse().map_err(|_| "bad --paper-code")?;
                i += 2;
            }
            "--bits" => {
                bits = need(i)?.parse().map_err(|_| "bad --bits")?;
                if !(1..=4).contains(&bits) {
                    return Err("--bits must be between 1 and 4".into());
                }
                i += 2;
            }
            "--no-pjl" => {
                pjl = false;
                i += 1;
            }
            "-o" | "--out" => {
                out = Some(need(i)?.clone());
                i += 2;
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }

    let bih_path = bih.ok_or("--bih is required")?;
    let out = out.ok_or("-o is required")?;
    if width == 0 || height == 0 {
        return Err("--width and --height are required".into());
    }
    let bih = read_file(&bih_path)?;
    if bih.len() != p1100_protocol::JBIG_BIH_LEN {
        return Err(format!(
            "{} is {} bytes; the JBIG BIH must be exactly {} bytes",
            bih_path,
            bih.len(),
            p1100_protocol::JBIG_BIH_LEN
        ));
    }
    let mut bid_data = Vec::new();
    for p in &bids {
        bid_data.push(read_file(p)?);
    }
    let bid_refs: Vec<&[u8]> = bid_data.iter().map(|v| v.as_slice()).collect();

    let mut g = PageGeometry::mono(width, height, resolution);
    g.paper = paper;
    g.bits_per_pixel = bits;

    let mut enc = Encoder::new();
    let mut file = Vec::new();
    if pjl {
        file.extend_from_slice(&pjl_prologue(&PjlSettings::default()));
    }
    enc.stream_signature();
    enc.doc_start(&JobOptions::default());
    enc.page_start(&g);
    enc.jbig_image(&bih, &bid_refs);
    enc.end_jbig(g.duplex);
    enc.page_end(g.duplex);
    enc.doc_end();
    file.extend_from_slice(enc.as_bytes());
    if pjl {
        file.extend_from_slice(&pjl_epilogue());
    }
    fs::write(&out, &file).map_err(|e| format!("cannot write {out}: {e}"))?;
    eprintln!(
        "wrote {} bytes to {out} (zjs body {}, {} data chunk(s))",
        file.len(),
        enc.as_bytes().len(),
        bid_refs.len()
    );
    Ok(())
}
