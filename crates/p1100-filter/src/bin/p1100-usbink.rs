//! CUPS `ReportLevels` command filter for the P1100 family's USB EWS interface.
//!
//! The printer exposes a second USB interface (class `ff/02/10`) that speaks HTTP.  HP's
//! original `usbink` uses it for `/DevMgmt/ConsumableConfigDyn.xml`; this ARM64 replacement
//! performs the same request and writes CUPS's standard marker attribute.

use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::process::ExitCode;

const REQUEST: &[u8] = b"GET /DevMgmt/ConsumableConfigDyn.xml HTTP/1.1\r\n\r\n";
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

fn usb_parts(uri: &str) -> Option<(String, String)> {
    let path = uri.strip_prefix("usb://")?.split_once('/')?.1;
    let (product, query) = path.split_once('?')?;
    let serial = query
        .split('&')
        .find_map(|part| part.strip_prefix("serial="))?;
    Some((decode_component(product)?, decode_component(serial)?))
}

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

fn marker_level(xml: &str) -> Option<u8> {
    let value = xml.split_once(PREFIX)?.1.split_once('<')?.0.trim();
    value.parse::<u8>().ok().filter(|level| *level <= 100)
}

fn decode_chunked_http(response: &str) -> Option<String> {
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

#[cfg(target_os = "macos")]
mod usb {
    use super::{decode_chunked_http, marker_level, REQUEST};
    use std::env;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::ptr;

    type KernReturn = i32;
    type IoObject = u32;
    type IoIterator = u32;
    type IoService = u32;
    type CfType = *const c_void;
    type CfMutableDictionary = *mut c_void;

    const KERN_SUCCESS: KernReturn = 0;
    const K_USB_IN: u8 = 1;
    const K_USB_BULK: u8 = 2;
    const PLUGIN_UUID: [u8; 16] = [
        0xc2, 0x44, 0xe8, 0x58, 0x10, 0x9c, 0x11, 0xd4, 0x91, 0xd4, 0x00, 0x50, 0xe4, 0xc6, 0x42,
        0x6f,
    ];
    const USB_CLIENT_UUID: [u8; 16] = [
        0x2d, 0x97, 0x86, 0xc6, 0x9e, 0xf3, 0x11, 0xd4, 0xad, 0x51, 0x00, 0x0a, 0x27, 0x05, 0x28,
        0x61,
    ];
    const USB_INTERFACE_UUID_182: [u8; 16] = [
        0x49, 0x23, 0xac, 0x4c, 0x48, 0x96, 0x11, 0xd5, 0x92, 0x08, 0x00, 0x0a, 0x27, 0x80, 0x1e,
        0x86,
    ];

    #[repr(C)]
    struct PluginVTable {
        _reserved: usize,
        query_interface: unsafe extern "C" fn(*mut c_void, [u8; 16], *mut *mut c_void) -> i32,
        _add_ref: usize,
        _release: usize,
    }

    #[repr(C)]
    struct UsbVTable {
        _reserved: usize,
        _query_interface: usize,
        _add_ref: usize,
        release: unsafe extern "C" fn(*mut c_void) -> u32,
        _async: [usize; 4],
        open: unsafe extern "C" fn(*mut c_void) -> i32,
        close: unsafe extern "C" fn(*mut c_void) -> i32,
        _before_pipe_properties: [usize; 16],
        pipe_properties: unsafe extern "C" fn(
            *mut c_void,
            u8,
            *mut u8,
            *mut u8,
            *mut u8,
            *mut u16,
            *mut u8,
        ) -> i32,
        _after_pipe_properties: [usize; 12],
        read_pipe_to: unsafe extern "C" fn(*mut c_void, u8, *mut c_void, *mut u32, u32, u32) -> i32,
        write_pipe_to: unsafe extern "C" fn(*mut c_void, u8, *mut c_void, u32, u32, u32) -> i32,
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> CfMutableDictionary;
        fn IOServiceGetMatchingServices(
            master: u32,
            matching: CfMutableDictionary,
            existing: *mut IoIterator,
        ) -> KernReturn;
        fn IOIteratorNext(iterator: IoIterator) -> IoService;
        fn IOObjectRelease(object: IoObject) -> KernReturn;
        fn IORegistryEntryCreateCFProperty(
            entry: IoService,
            key: CfType,
            allocator: CfType,
            options: u32,
        ) -> CfType;
        fn IOCreatePlugInInterfaceForService(
            service: IoService,
            plugin_type: CfType,
            interface_type: CfType,
            plugin: *mut *mut *mut PluginVTable,
            score: *mut i32,
        ) -> i32;
        fn IODestroyPlugInInterface(plugin: *mut *mut PluginVTable) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFAllocatorDefault: CfType;
        fn CFStringCreateWithCString(
            allocator: CfType,
            value: *const c_char,
            encoding: u32,
        ) -> CfType;
        fn CFStringGetCString(
            value: CfType,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> bool;
        fn CFNumberGetValue(value: CfType, number_type: i32, output: *mut i32) -> bool;
        fn CFRelease(value: CfType);
        fn CFUUIDGetConstantUUIDWithBytes(
            allocator: CfType,
            b0: u8,
            b1: u8,
            b2: u8,
            b3: u8,
            b4: u8,
            b5: u8,
            b6: u8,
            b7: u8,
            b8: u8,
            b9: u8,
            b10: u8,
            b11: u8,
            b12: u8,
            b13: u8,
            b14: u8,
            b15: u8,
        ) -> CfType;
    }

    #[link(name = "IOUSBHost", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    const UTF8: u32 = 0x0800_0100;

    fn trace(message: &str) {
        if env::var_os("P1100_USBINK_DEBUG").is_some() {
            eprintln!("p1100-usbink: {message}");
        }
    }

    type ObjcId = *mut c_void;

    unsafe fn selector(name: &str) -> *mut c_void {
        let name = CString::new(name).unwrap();
        sel_registerName(name.as_ptr())
    }

    unsafe fn message0(receiver: ObjcId, name: &str) -> ObjcId {
        let send: unsafe extern "C" fn(ObjcId, *mut c_void) -> ObjcId =
            std::mem::transmute(objc_msgSend as *const ());
        send(receiver, selector(name))
    }

    unsafe fn message_init(receiver: ObjcId, service: IoService, error: *mut ObjcId) -> ObjcId {
        let send: unsafe extern "C" fn(
            ObjcId,
            *mut c_void,
            IoService,
            ObjcId,
            *mut ObjcId,
            ObjcId,
        ) -> ObjcId = std::mem::transmute(objc_msgSend as *const ());
        send(
            receiver,
            selector("initWithIOService:queue:error:interestHandler:"),
            service,
            ptr::null_mut(),
            error,
            ptr::null_mut(),
        )
    }

    unsafe fn message_void(receiver: ObjcId, name: &str) {
        let send: unsafe extern "C" fn(ObjcId, *mut c_void) =
            std::mem::transmute(objc_msgSend as *const ());
        send(receiver, selector(name));
    }

    unsafe fn message_integer(receiver: ObjcId, name: &str) -> isize {
        let send: unsafe extern "C" fn(ObjcId, *mut c_void) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        send(receiver, selector(name))
    }

    unsafe fn modern_session(service: IoService) -> Result<ObjcId, String> {
        let class_name = CString::new("IOUSBHostInterface").unwrap();
        let class = objc_getClass(class_name.as_ptr());
        if class.is_null() {
            return Err("IOUSBHost is unavailable".into());
        }
        let allocated = message0(class, "alloc");
        let mut error = ptr::null_mut();
        let interface = message_init(allocated, service, &mut error);
        if interface.is_null() {
            let detail = if error.is_null() {
                "unknown error".into()
            } else {
                format!("IOReturn {}", message_integer(error, "code"))
            };
            if !error.is_null() {
                message_void(error, "release");
            }
            return Err(format!("IOUSBHost cannot open HP EWS: {detail}"));
        }
        Ok(interface)
    }

    unsafe fn message_pipe(receiver: ObjcId, address: usize, error: *mut ObjcId) -> ObjcId {
        let send: unsafe extern "C" fn(ObjcId, *mut c_void, usize, *mut ObjcId) -> ObjcId =
            std::mem::transmute(objc_msgSend as *const ());
        send(
            receiver,
            selector("copyPipeWithAddress:error:"),
            address,
            error,
        )
    }

    unsafe fn mutable_data(bytes: Option<&[u8]>, length: usize) -> ObjcId {
        let class_name = CString::new("NSMutableData").unwrap();
        let class = objc_getClass(class_name.as_ptr());
        let allocated = message0(class, "alloc");
        if let Some(bytes) = bytes {
            let send: unsafe extern "C" fn(ObjcId, *mut c_void, *const u8, usize) -> ObjcId =
                std::mem::transmute(objc_msgSend as *const ());
            send(
                allocated,
                selector("initWithBytes:length:"),
                bytes.as_ptr(),
                bytes.len(),
            )
        } else {
            let send: unsafe extern "C" fn(ObjcId, *mut c_void, usize) -> ObjcId =
                std::mem::transmute(objc_msgSend as *const ());
            send(allocated, selector("initWithLength:"), length)
        }
    }

    unsafe fn message_send(
        pipe: ObjcId,
        data: ObjcId,
        transferred: *mut usize,
        error: *mut ObjcId,
    ) -> bool {
        let send: unsafe extern "C" fn(
            ObjcId,
            *mut c_void,
            ObjcId,
            *mut usize,
            f64,
            *mut ObjcId,
        ) -> i8 = std::mem::transmute(objc_msgSend as *const ());
        send(
            pipe,
            selector("sendIORequestWithData:bytesTransferred:completionTimeout:error:"),
            data,
            transferred,
            1.0,
            error,
        ) != 0
    }

    unsafe fn data_bytes(data: ObjcId) -> *const u8 {
        let send: unsafe extern "C" fn(ObjcId, *mut c_void) -> *const u8 =
            std::mem::transmute(objc_msgSend as *const ());
        send(data, selector("bytes"))
    }

    unsafe fn ews_pipes(interface: ObjcId) -> Result<(ObjcId, ObjcId), String> {
        let mut input: ObjcId = ptr::null_mut();
        let mut output: ObjcId = ptr::null_mut();
        for address in (1usize..=15).chain(0x81..=0x8f) {
            let mut error = ptr::null_mut();
            let pipe = message_pipe(interface, address, &mut error);
            if !error.is_null() {
                message_void(error, "release");
            }
            if pipe.is_null() {
                continue;
            }
            // The EWS interface has exactly one usable IN and OUT pipe.  Asking an
            // IOUSBHostPipe for its legacy descriptor makes IOUSBLib crash when CUPS
            // owns the printer backend, so select the first pipe in each direction.
            if address & 0x80 == 0 && output.is_null() {
                output = pipe;
                continue;
            }
            if address & 0x80 != 0 && input.is_null() {
                input = pipe;
                continue;
            }
            message_void(pipe, "release");
        }
        if input.is_null() || output.is_null() {
            if !input.is_null() {
                message_void(input, "release");
            }
            if !output.is_null() {
                message_void(output, "release");
            }
            return Err("HP EWS did not expose both bulk pipes".into());
        }
        Ok((input, output))
    }

    unsafe fn modern_level(service: IoService) -> Result<Option<u8>, String> {
        let interface = modern_session(service)?;
        trace("opened HP EWS through IOUSBHost");
        let result = (|| {
            let (input, output) = ews_pipes(interface)?;
            let request = mutable_data(Some(REQUEST), 0);
            let mut sent = 0;
            let mut error = ptr::null_mut();
            let wrote = message_send(output, request, &mut sent, &mut error);
            message_void(request, "release");
            message_void(output, "release");
            if !error.is_null() {
                message_void(error, "release");
            }
            if !wrote || sent != REQUEST.len() {
                message_void(input, "release");
                return Err("HP EWS rejected the consumable request".into());
            }
            let response = mutable_data(None, 32768);
            let mut received = 0;
            let mut error = ptr::null_mut();
            let read = message_send(input, response, &mut received, &mut error);
            message_void(input, "release");
            if !error.is_null() {
                message_void(error, "release");
            }
            let body = if read {
                std::str::from_utf8(std::slice::from_raw_parts(data_bytes(response), received)).ok()
            } else {
                None
            };
            if env::var_os("P1100_USBINK_DEBUG").is_some() {
                match body {
                    Some(body) => eprintln!("p1100-usbink: received {received} bytes: {body}"),
                    None => {
                        eprintln!("p1100-usbink: USB response was not UTF-8 ({received} bytes)")
                    }
                }
            }
            let result = body
                .and_then(decode_chunked_http)
                .and_then(|xml| marker_level(&xml));
            message_void(response, "release");
            Ok(result)
        })();
        message_void(interface, "destroy");
        message_void(interface, "release");
        result
    }

    unsafe fn uuid(bytes: [u8; 16]) -> CfType {
        CFUUIDGetConstantUUIDWithBytes(
            ptr::null(),
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
            bytes[4],
            bytes[5],
            bytes[6],
            bytes[7],
            bytes[8],
            bytes[9],
            bytes[10],
            bytes[11],
            bytes[12],
            bytes[13],
            bytes[14],
            bytes[15],
        )
    }

    unsafe fn legacy_level(service: IoService) -> Result<Option<u8>, String> {
        let mut plugin: *mut *mut PluginVTable = ptr::null_mut();
        let mut score = 0;
        if IOCreatePlugInInterfaceForService(
            service,
            uuid(USB_CLIENT_UUID),
            uuid(PLUGIN_UUID),
            &mut plugin,
            &mut score,
        ) != KERN_SUCCESS
            || plugin.is_null()
        {
            return Err("IOKit cannot create the HP EWS plugin".into());
        }
        trace("created HP EWS plugin");
        let mut interface: *mut *mut UsbVTable = ptr::null_mut();
        let queried = ((**plugin).query_interface)(
            plugin as *mut c_void,
            USB_INTERFACE_UUID_182,
            &mut interface as *mut _ as *mut *mut c_void,
        );
        IODestroyPlugInInterface(plugin);
        if queried != KERN_SUCCESS || interface.is_null() {
            return Err("IOKit cannot create the HP EWS interface".into());
        }
        trace("created HP EWS legacy interface");
        let vtable = *interface;
        let result = (|| {
            if ((*vtable).open)(interface as *mut c_void) != KERN_SUCCESS {
                return Err("IOKit cannot open the HP EWS interface".into());
            }
            trace("opened HP EWS legacy interface");
            let mut input = 0;
            let mut output = 0;
            for pipe in 1..=3 {
                let (mut direction, mut number, mut kind, mut packet, mut interval) =
                    (0, 0, 0, 0, 0);
                let result = ((*vtable).pipe_properties)(
                    interface as *mut c_void,
                    pipe,
                    &mut direction,
                    &mut number,
                    &mut kind,
                    &mut packet,
                    &mut interval,
                );
                trace(&format!("legacy pipe {pipe}: result={result} direction={direction} number={number} type={kind}"));
                if result == KERN_SUCCESS && kind == K_USB_BULK {
                    if direction == K_USB_IN {
                        input = pipe;
                    } else {
                        output = pipe;
                    }
                }
            }
            if input == 0 || output == 0 {
                ((*vtable).close)(interface as *mut c_void);
                return Err("HP EWS did not expose both bulk pipes".into());
            }
            let wrote = ((*vtable).write_pipe_to)(
                interface as *mut c_void,
                output,
                REQUEST.as_ptr() as *mut c_void,
                REQUEST.len() as u32,
                1000,
                1000,
            );
            trace(&format!("legacy request write result={wrote}"));
            if wrote != KERN_SUCCESS {
                ((*vtable).close)(interface as *mut c_void);
                return Err("HP EWS rejected the consumable request".into());
            }
            let mut response = [0u8; 32768];
            let mut received = response.len() as u32;
            let read = ((*vtable).read_pipe_to)(
                interface as *mut c_void,
                input,
                response.as_mut_ptr() as *mut c_void,
                &mut received,
                1000,
                1000,
            );
            ((*vtable).close)(interface as *mut c_void);
            if read != KERN_SUCCESS {
                return Err("HP EWS did not return consumable data".into());
            }
            let body = std::str::from_utf8(&response[..received as usize]).ok();
            if env::var_os("P1100_USBINK_DEBUG").is_some() {
                match body {
                    Some(body) => eprintln!("p1100-usbink: received {received} bytes: {body}"),
                    None => {
                        eprintln!("p1100-usbink: USB response was not UTF-8 ({received} bytes)")
                    }
                }
            }
            Ok(body
                .and_then(decode_chunked_http)
                .and_then(|xml| marker_level(&xml)))
        })();
        ((*vtable).release)(interface as *mut c_void);
        result
    }

    unsafe fn property(service: IoService, name: &str) -> Option<String> {
        let name = CString::new(name).ok()?;
        let key = CFStringCreateWithCString(kCFAllocatorDefault, name.as_ptr(), UTF8);
        if key.is_null() {
            return None;
        }
        let value = IORegistryEntryCreateCFProperty(service, key, kCFAllocatorDefault, 0);
        CFRelease(key);
        if value.is_null() {
            return None;
        }
        let mut buffer = [0i8; 256];
        let ok = CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as isize, UTF8);
        CFRelease(value);
        ok.then(|| {
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        })
    }

    unsafe fn number_property(service: IoService, name: &str) -> Option<i32> {
        let name = CString::new(name).ok()?;
        let key = CFStringCreateWithCString(kCFAllocatorDefault, name.as_ptr(), UTF8);
        if key.is_null() {
            return None;
        }
        let value = IORegistryEntryCreateCFProperty(service, key, kCFAllocatorDefault, 0);
        CFRelease(key);
        if value.is_null() {
            return None;
        }
        let mut output = 0;
        let ok = CFNumberGetValue(value, 3, &mut output); // kCFNumberSInt32Type
        CFRelease(value);
        ok.then_some(output)
    }

    pub fn level(product: &str, serial: &str) -> Result<Option<u8>, String> {
        unsafe {
            trace("enumerating USB interfaces");
            let class = CString::new("IOUSBHostInterface").unwrap();
            let matching = IOServiceMatching(class.as_ptr());
            let mut iterator = 0;
            if matching.is_null()
                || IOServiceGetMatchingServices(0, matching, &mut iterator) != KERN_SUCCESS
            {
                return Err("cannot enumerate USB interfaces".into());
            }
            let mut service = 0;
            loop {
                let candidate = IOIteratorNext(iterator);
                if candidate == 0 {
                    break;
                }
                let found = property(candidate, "USB Product Name").as_deref() == Some(product)
                    && property(candidate, "USB Serial Number").as_deref() == Some(serial)
                    && number_property(candidate, "bInterfaceClass") == Some(0xff)
                    && number_property(candidate, "bInterfaceSubClass") == Some(2)
                    && number_property(candidate, "bInterfaceProtocol") == Some(16);
                if found {
                    trace("found HP EWS");
                    service = candidate;
                    break;
                }
                IOObjectRelease(candidate);
            }
            IOObjectRelease(iterator);
            if service == 0 {
                trace("HP EWS was not found");
                return Ok(None);
            }
            let result = if env::var_os("CUPS_SERVER").is_some() {
                trace("opening HP EWS through legacy IOKit for CUPS");
                legacy_level(service)
            } else {
                modern_level(service)
            };
            IOObjectRelease(service);
            result
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod usb {
    pub fn level(_: &str, _: &str) -> Result<Option<u8>, String> {
        Err("USB consumable queries require macOS".into())
    }
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
