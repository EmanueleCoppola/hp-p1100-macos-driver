//! Reading the consumable level through IOUSBHost, the modern USB stack.

use super::ffi::*;
use crate::http::{decode_chunked_http, marker_level, REQUEST};
use std::env;
use std::ffi::{c_void, CString};
use std::ptr;

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

pub(super) unsafe fn modern_level(service: IoService) -> Result<Option<u8>, String> {
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
