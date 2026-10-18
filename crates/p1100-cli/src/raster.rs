//! The two commands that read a raster stream: reporting what is in it, and converting it
//! into the 1-bit bitmap the printer is fed.

use std::fs;

use p1100_raster::{to_mono1, RasterReader, DEFAULT_THRESHOLD};

use crate::read_file;

pub(crate) fn raster_info(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("raster-info needs a file")?;
    let data = read_file(path)?;
    let mut reader = RasterReader::new(&data);
    let mut n = 0;
    while let Some((h, px)) = reader.next_page().map_err(|e| format!("{e:?}"))? {
        n += 1;
        println!(
            "page {n}: {}x{} px, {} bpc/{} bpp, {} bytes/line, {} colour(s), page {:?}",
            h.cups_width,
            h.cups_height,
            h.cups_bits_per_color,
            h.cups_bits_per_pixel,
            h.cups_bytes_per_line,
            h.cups_num_colors,
            h.cups_page_size_name
        );
        println!(
            "        media type {:?}, resolution {:?} dpi, margins {:?} pt, copies {}",
            h.media_type, h.hw_resolution, h.margins, h.num_copies
        );
        println!(
            "        mono row stride {} bytes (128-bit aligned), JBIG width {} (128 px aligned), payload {} bytes",
            h.mono_row_bytes(),
            h.jbig_width(),
            px.len()
        );
        println!(
            "        page size {:?} pt, bbox {:?}, borderless {}, cupsCompression {}",
            h.cups_page_size, h.cups_imaging_bbox, h.cups_borderless_scaling, h.cups_compression
        );
    }
    println!("pages: {n}");
    Ok(())
}
pub(crate) fn raster_to_mono1(args: &[String]) -> Result<(), String> {
    let (inp, outp) = match args {
        [i, o] => (i, o),
        _ => return Err("raster-to-mono1 needs <in> <out>".into()),
    };
    let data = read_file(inp)?;
    let mut reader = RasterReader::new(&data);
    let mut all = Vec::new();
    let mut pages = 0;
    while let Some((h, px)) = reader.next_page().map_err(|e| format!("{e:?}"))? {
        pages += 1;
        let mono = to_mono1(&h, px, DEFAULT_THRESHOLD).map_err(|e| format!("{e:?}"))?;
        eprintln!(
            "page {pages}: {}x{} -> {} bytes of 1bpp (stride {})",
            h.cups_width,
            h.cups_height,
            mono.len(),
            h.mono_row_bytes()
        );
        all.extend_from_slice(&mono);
    }
    fs::write(outp, &all).map_err(|e| format!("cannot write {outp}: {e}"))?;
    eprintln!("wrote {} bytes to {outp} ({pages} page(s))", all.len());
    Ok(())
}
