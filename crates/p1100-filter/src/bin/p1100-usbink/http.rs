//! The HTTP request to the printer's embedded web server, and the little of the response
//! this filter needs.

pub(crate) const REQUEST: &[u8] = b"GET /DevMgmt/ConsumableConfigDyn.xml HTTP/1.1\r\n\r\n";
const PREFIX: &str = "<dd:ConsumablePercentageLevelRemaining>";
fn decode_component(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            out.push(byte);
        }
    }
    String::from_utf8(out).ok()
}
pub(crate) fn usb_parts(uri: &str) -> Option<(String, String)> {
    let path = uri.strip_prefix("usb://")?.split_once('/')?.1;
    let (product, query) = path.split_once('?')?;
    let serial = query
        .split('&')
        .find_map(|part| part.strip_prefix("serial="))?;
    Some((decode_component(product)?, decode_component(serial)?))
}
pub(crate) fn marker_level(xml: &str) -> Option<u8> {
    let value = xml.split_once(PREFIX)?.1.split_once('<')?.0.trim();
    value.parse::<u8>().ok().filter(|level| *level <= 100)
}
pub(crate) fn decode_chunked_http(response: &str) -> Option<String> {
    let (_, mut input) = response.split_once("\r\n\r\n")?;
    let mut body = String::new();
    loop {
        let (size, rest) = input.split_once("\r\n")?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            return Some(body);
        }
        if rest.len() < size + 2 || &rest[size..size + 2] != "\r\n" {
            return None;
        }
        body.push_str(&rest[..size]);
        input = &rest[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usb_uri_and_percent_encoding() {
        assert_eq!(
            usb_parts("usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=ABC%201"),
            Some(("HP LaserJet Professional P1102".into(), "ABC 1".into()))
        );
    }
    #[test]
    fn accepts_only_valid_marker_percentages() {
        assert_eq!(
            marker_level(
                "<dd:ConsumablePercentageLevelRemaining>42</dd:ConsumablePercentageLevelRemaining>"
            ),
            Some(42)
        );
        assert_eq!(marker_level("<dd:ConsumablePercentageLevelRemaining>101</dd:ConsumablePercentageLevelRemaining>"), None);
    }
    #[test]
    fn reassembles_chunked_xml_before_reading_the_level() {
        let first = "<dd:ConsumablePercentageLevelRemaining>20</dd:ConsumablePerc";
        let second = "entageLevelRemaining>";
        let response = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{first}\r\n{:x}\r\n{second}\r\n0\r\n\r\n", first.len(), second.len());
        assert_eq!(
            decode_chunked_http(&response)
                .as_deref()
                .and_then(marker_level),
            Some(20)
        );
    }
}
