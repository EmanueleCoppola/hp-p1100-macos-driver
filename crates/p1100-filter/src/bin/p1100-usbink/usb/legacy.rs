//! Reading the consumable level through the legacy IOUSBLib plugin interface, which is the
//! path HP's own filter uses when CUPS owns the printer backend.

use super::ffi::*;
use crate::http::{decode_chunked_http, marker_level, REQUEST};
use std::env;
use std::ffi::c_void;
use std::ptr;

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
pub(super) unsafe fn legacy_level(service: IoService) -> Result<Option<u8>, String> {
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
            let (mut direction, mut number, mut kind, mut packet, mut interval) = (0, 0, 0, 0, 0);
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
