//! The IOKit, CoreFoundation and Objective-C declarations the USB paths need.
//!
//! They are grouped here so the two ways of reaching the printer cannot each grow their own
//! copy, and so the raw declarations are visibly separate from the code that uses them.

use std::env;
use std::ffi::{c_char, c_void};

pub(super) type KernReturn = i32;
pub(super) type IoObject = u32;
pub(super) type IoIterator = u32;
pub(super) type IoService = u32;
pub(super) type CfType = *const c_void;
pub(super) type CfMutableDictionary = *mut c_void;
pub(super) const KERN_SUCCESS: KernReturn = 0;
pub(super) const K_USB_IN: u8 = 1;
pub(super) const K_USB_BULK: u8 = 2;
pub(super) const PLUGIN_UUID: [u8; 16] = [
    0xc2, 0x44, 0xe8, 0x58, 0x10, 0x9c, 0x11, 0xd4, 0x91, 0xd4, 0x00, 0x50, 0xe4, 0xc6, 0x42, 0x6f,
];
pub(super) const USB_CLIENT_UUID: [u8; 16] = [
    0x2d, 0x97, 0x86, 0xc6, 0x9e, 0xf3, 0x11, 0xd4, 0xad, 0x51, 0x00, 0x0a, 0x27, 0x05, 0x28, 0x61,
];
pub(super) const USB_INTERFACE_UUID_182: [u8; 16] = [
    0x49, 0x23, 0xac, 0x4c, 0x48, 0x96, 0x11, 0xd5, 0x92, 0x08, 0x00, 0x0a, 0x27, 0x80, 0x1e, 0x86,
];

#[repr(C)]
pub(super) struct PluginVTable {
    _reserved: usize,
    pub(super) query_interface:
        unsafe extern "C" fn(*mut c_void, [u8; 16], *mut *mut c_void) -> i32,
    _add_ref: usize,
    _release: usize,
}

#[repr(C)]
pub(super) struct UsbVTable {
    _reserved: usize,
    _query_interface: usize,
    _add_ref: usize,
    pub(super) release: unsafe extern "C" fn(*mut c_void) -> u32,
    _async: [usize; 4],
    pub(super) open: unsafe extern "C" fn(*mut c_void) -> i32,
    pub(super) close: unsafe extern "C" fn(*mut c_void) -> i32,
    _before_pipe_properties: [usize; 16],
    pub(super) pipe_properties:
        unsafe extern "C" fn(*mut c_void, u8, *mut u8, *mut u8, *mut u8, *mut u16, *mut u8) -> i32,
    _after_pipe_properties: [usize; 12],
    pub(super) read_pipe_to:
        unsafe extern "C" fn(*mut c_void, u8, *mut c_void, *mut u32, u32, u32) -> i32,
    pub(super) write_pipe_to:
        unsafe extern "C" fn(*mut c_void, u8, *mut c_void, u32, u32, u32) -> i32,
}
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    pub(super) fn IOServiceMatching(name: *const c_char) -> CfMutableDictionary;
    pub(super) fn IOServiceGetMatchingServices(
        master: u32,
        matching: CfMutableDictionary,
        existing: *mut IoIterator,
    ) -> KernReturn;
    pub(super) fn IOIteratorNext(iterator: IoIterator) -> IoService;
    pub(super) fn IOObjectRelease(object: IoObject) -> KernReturn;
    pub(super) fn IORegistryEntryCreateCFProperty(
        entry: IoService,
        key: CfType,
        allocator: CfType,
        options: u32,
    ) -> CfType;
    pub(super) fn IOCreatePlugInInterfaceForService(
        service: IoService,
        plugin_type: CfType,
        interface_type: CfType,
        plugin: *mut *mut *mut PluginVTable,
        score: *mut i32,
    ) -> i32;
    pub(super) fn IODestroyPlugInInterface(plugin: *mut *mut PluginVTable) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub(super) static kCFAllocatorDefault: CfType;
    pub(super) fn CFStringCreateWithCString(
        allocator: CfType,
        value: *const c_char,
        encoding: u32,
    ) -> CfType;
    pub(super) fn CFStringGetCString(
        value: CfType,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> bool;
    pub(super) fn CFNumberGetValue(value: CfType, number_type: i32, output: *mut i32) -> bool;
    pub(super) fn CFRelease(value: CfType);
    pub(super) fn CFUUIDGetConstantUUIDWithBytes(
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
    pub(super) fn objc_getClass(name: *const c_char) -> *mut c_void;
    pub(super) fn sel_registerName(name: *const c_char) -> *mut c_void;
    pub(super) fn objc_msgSend();
}
pub(super) const UTF8: u32 = 0x0800_0100;

pub(super) fn trace(message: &str) {
    if env::var_os("P1100_USBINK_DEBUG").is_some() {
        eprintln!("p1100-usbink: {message}");
    }
}
