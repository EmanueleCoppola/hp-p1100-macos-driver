//! Which USB interface the printer exposes the embedded web server on, and how to reach it.
//!
//! HP's printer shows a second interface (class `ff/02/10`) beside the printing one. This
//! module finds it by matching the product name and serial number the CUPS device URI carries,
//! then asks it for the consumable level through one of two stacks: IOUSBHost, or the legacy
//! IOUSBLib plugin interface when CUPS owns the printer backend.

#[cfg(target_os = "macos")]
mod ffi;
#[cfg(target_os = "macos")]
mod legacy;
#[cfg(target_os = "macos")]
mod modern;

#[cfg(target_os = "macos")]
use self::ffi::*;
#[cfg(target_os = "macos")]
use self::legacy::legacy_level;
#[cfg(target_os = "macos")]
use self::modern::modern_level;
#[cfg(target_os = "macos")]
use std::env;
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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

#[cfg(not(target_os = "macos"))]
pub fn level(_: &str, _: &str) -> Result<Option<u8>, String> {
    Err("USB consumable queries require macOS".into())
}
