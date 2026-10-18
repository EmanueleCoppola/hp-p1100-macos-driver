//! CUPS `ReportLevels` command filter for the P1100 family's USB EWS interface.
//!
//! The printer exposes a second USB interface (class `ff/02/10`) that speaks HTTP.  HP's
//! original `usbink` uses it for `/DevMgmt/ConsumableConfigDyn.xml`; this ARM64 replacement
//! performs the same request and writes CUPS's standard marker attribute.

mod http;
mod usb;

use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::process::ExitCode;

use crate::http::usb_parts;

fn report_levels(mut input: impl BufRead) -> bool {
    let mut line = String::new();
    while input.read_line(&mut line).unwrap_or(0) != 0 {
        if line.trim().eq_ignore_ascii_case("ReportLevels") {
            return true;
        }
        line.clear();
    }
    false
}
fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    let input: Box<dyn BufRead> = if args.len() == 7 {
        match File::open(&args[6]) {
            Ok(file) => Box::new(BufReader::new(file)),
            Err(error) => return fail(&format!("cannot open command file: {error}")),
        }
    } else {
        Box::new(BufReader::new(io::stdin()))
    };
    if !report_levels(input) {
        return fail("unknown printer command");
    }
    let Some(uri) = env::var("DEVICE_URI").ok() else {
        return fail("DEVICE_URI is not set");
    };
    let Some((product, serial)) = usb_parts(&uri) else {
        return fail("DEVICE_URI is not a USB URI with a serial number");
    };
    match usb::level(&product, &serial) {
        Ok(Some(level)) => {
            eprintln!("ATTR: marker-colors=#000000 marker-levels={level} marker-names=Black marker-types=toner");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            eprintln!("p1100-usbink: no consumable level returned by {product} ({serial})");
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}
fn fail(message: &str) -> ExitCode {
    eprintln!("p1100-usbink: {message}");
    ExitCode::from(1)
}
