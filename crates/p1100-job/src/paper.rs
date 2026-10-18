//! The page size CUPS named, as the ~~DMPAPER~~ code the printer expects.

use p1100_protocol::paper;

/// Map a CUPS page size name to a `DMPAPER` code.
///
/// CUPS puts the PPD's `*PageSize` choice name in `cupsPageSizeName` - measured by running
/// every size this PPD declares through cupsfilter - so these are exactly the names that
/// arrive. The codes are the ones the reference driver uses for the same page sizes (its
/// wrapper keys on these same names). Each entry was checked against it by page geometry
/// rather than by name, which is how `EnvISOB5` lands on the envelope code 34 instead of ISO
/// B5 paper's 259, and `8.5x13Custom` on the German legal fanfold code 258 rather than the
/// custom code, because 612x936 pt is that size.
pub fn paper_code(page_size_name: &str) -> Option<u32> {
    let name = page_size_name.trim();
    let code = if name.eq_ignore_ascii_case("letter") {
        paper::LETTER
    } else if name.eq_ignore_ascii_case("legal") {
        paper::LEGAL
    } else if name.eq_ignore_ascii_case("executive") {
        paper::EXECUTIVE
    } else if name.eq_ignore_ascii_case("a3") {
        paper::A3
    } else if name.eq_ignore_ascii_case("a4") {
        paper::A4
    } else if name.eq_ignore_ascii_case("a5") {
        paper::A5
    } else if name.eq_ignore_ascii_case("a6") {
        paper::A6
    } else if name.eq_ignore_ascii_case("b5") || name.eq_ignore_ascii_case("b5jis") {
        paper::B5
    } else if name.eq_ignore_ascii_case("env10") || name.eq_ignore_ascii_case("env#10") {
        paper::ENV_10
    } else if name.eq_ignore_ascii_case("envdl")
        || name.eq_ignore_ascii_case("env dl")
        || name.eq_ignore_ascii_case("dl")
    {
        paper::ENV_DL
    } else if name.eq_ignore_ascii_case("envc5") {
        paper::ENV_C5
    } else if name.eq_ignore_ascii_case("envisob5") || name.eq_ignore_ascii_case("envb5") {
        paper::ENV_B5
    } else if name.eq_ignore_ascii_case("monarch") {
        paper::ENV_MONARCH
    } else if name.eq_ignore_ascii_case("postcard") {
        paper::POSTCARD
    } else if name.eq_ignore_ascii_case("doublepostcard") {
        paper::DOUBLE_POSTCARD
    } else if name.eq_ignore_ascii_case("16k197x273mm") {
        paper::K16_197X273
    } else if name.eq_ignore_ascii_case("16k184x260mm") {
        paper::K16_184X260
    } else if name.eq_ignore_ascii_case("16k195x270mm") {
        paper::K16_195X270
    } else if name.eq_ignore_ascii_case("8.5x13custom") {
        paper::FANFOLD_GERMAN_LEGAL
    } else if name.eq_ignore_ascii_case("custom") {
        paper::CUSTOM
    } else {
        return None;
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_code_follows_the_cups_page_size_name() {
        assert_eq!(paper_code("Letter"), Some(paper::LETTER));
        assert_eq!(paper_code("letter"), Some(paper::LETTER));
        assert_eq!(paper_code("A4"), Some(paper::A4));
        assert_eq!(paper_code("A3"), Some(paper::A3));
        assert_eq!(paper_code("Legal"), Some(paper::LEGAL));
        assert_eq!(paper_code("Executive"), Some(paper::EXECUTIVE));
        assert_eq!(paper_code("A5"), Some(paper::A5));
        assert_eq!(paper_code("Env DL"), Some(paper::ENV_DL));
        assert_eq!(paper_code("Custom.100x200"), None);
    }

    /// Every `*PageSize` the printer's PPD declares must map to a code, and to the code the
    /// reference driver uses for that page size. A name that falls through here would send
    /// Letter for a page that is not Letter, which is what this test exists to prevent: the
    /// list below is the PPD's own option list, and 8 of these names used to miss the table.
    #[test]
    fn every_page_size_the_ppd_declares_maps_to_the_reference_code() {
        let ppd_names = [
            ("Letter", paper::LETTER),
            ("Legal", paper::LEGAL),
            ("Executive", paper::EXECUTIVE),
            ("A4", paper::A4),
            ("A5", paper::A5),
            ("A6", paper::A6),
            ("B5JIS", paper::B5),
            ("Env10", paper::ENV_10),
            ("EnvDL", paper::ENV_DL),
            ("EnvC5", paper::ENV_C5),
            ("EnvISOB5", paper::ENV_B5),
            ("Monarch", paper::ENV_MONARCH),
            ("Postcard", paper::POSTCARD),
            ("DoublePostcard", paper::DOUBLE_POSTCARD),
            ("16K197x273mm", paper::K16_197X273),
            ("16K184x260mm", paper::K16_184X260),
            ("16K195x270mm", paper::K16_195X270),
            ("8.5x13Custom", paper::FANFOLD_GERMAN_LEGAL),
        ];
        for (name, code) in ppd_names {
            assert_eq!(paper_code(name), Some(code), "page size name {name}");
        }
        // A custom size arrives as this rather than as a PPD choice.
        assert_eq!(paper_code("Custom"), Some(paper::CUSTOM));
        // Names that are not page sizes at all still fall through to the caller's default.
        assert_eq!(paper_code("Custom.100x200"), None);
        assert_eq!(paper_code(""), None);
    }
}
