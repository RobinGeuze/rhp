//! The part of PCRE2's 8-bit API rhp calls, declared by hand from pcre2.h.
//! These signatures and constants are PCRE2's stable ABI (unchanged since
//! 10.30). build.rs links the library; see docs/design/libraries.md.

#![allow(non_camel_case_types, dead_code)]

use libc::{c_int, c_void, size_t};

pub type PCRE2_SIZE = size_t;
pub type PCRE2_SPTR = *const u8;

#[repr(C)]
pub struct pcre2_code_8 {
    _p: [u8; 0],
}
#[repr(C)]
pub struct pcre2_compile_context_8 {
    _p: [u8; 0],
}
#[repr(C)]
pub struct pcre2_general_context_8 {
    _p: [u8; 0],
}
#[repr(C)]
pub struct pcre2_jit_stack_8 {
    _p: [u8; 0],
}
#[repr(C)]
pub struct pcre2_match_context_8 {
    _p: [u8; 0],
}
#[repr(C)]
pub struct pcre2_match_data_8 {
    _p: [u8; 0],
}

pub type pcre2_jit_callback_8 = Option<unsafe extern "C" fn(*mut c_void) -> *mut pcre2_jit_stack_8>;

extern "C" {
    pub fn pcre2_config_8(what: u32, where_: *mut c_void) -> c_int;

    pub fn pcre2_compile_context_create_8(
        gcontext: *mut pcre2_general_context_8,
    ) -> *mut pcre2_compile_context_8;
    pub fn pcre2_compile_context_free_8(ccontext: *mut pcre2_compile_context_8);
    pub fn pcre2_set_compile_extra_options_8(
        ccontext: *mut pcre2_compile_context_8,
        extra_options: u32,
    ) -> c_int;

    pub fn pcre2_compile_8(
        pattern: PCRE2_SPTR,
        length: PCRE2_SIZE,
        options: u32,
        errorcode: *mut c_int,
        erroroffset: *mut PCRE2_SIZE,
        ccontext: *mut pcre2_compile_context_8,
    ) -> *mut pcre2_code_8;
    pub fn pcre2_code_free_8(code: *mut pcre2_code_8);
    pub fn pcre2_get_error_message_8(
        errorcode: c_int,
        buffer: *mut u8,
        bufflen: PCRE2_SIZE,
    ) -> c_int;
    pub fn pcre2_pattern_info_8(code: *const pcre2_code_8, what: u32, where_: *mut c_void)
        -> c_int;

    pub fn pcre2_jit_compile_8(code: *mut pcre2_code_8, options: u32) -> c_int;
    pub fn pcre2_jit_stack_create_8(
        startsize: PCRE2_SIZE,
        maxsize: PCRE2_SIZE,
        gcontext: *mut pcre2_general_context_8,
    ) -> *mut pcre2_jit_stack_8;
    pub fn pcre2_jit_stack_free_8(jit_stack: *mut pcre2_jit_stack_8);
    pub fn pcre2_jit_stack_assign_8(
        mcontext: *mut pcre2_match_context_8,
        callback_function: pcre2_jit_callback_8,
        callback_data: *mut c_void,
    );

    pub fn pcre2_match_context_create_8(
        gcontext: *mut pcre2_general_context_8,
    ) -> *mut pcre2_match_context_8;
    pub fn pcre2_match_context_free_8(mcontext: *mut pcre2_match_context_8);
    pub fn pcre2_set_match_limit_8(mcontext: *mut pcre2_match_context_8, value: u32) -> c_int;
    pub fn pcre2_set_depth_limit_8(mcontext: *mut pcre2_match_context_8, value: u32) -> c_int;

    pub fn pcre2_match_data_create_from_pattern_8(
        code: *const pcre2_code_8,
        gcontext: *mut pcre2_general_context_8,
    ) -> *mut pcre2_match_data_8;
    pub fn pcre2_match_data_free_8(match_data: *mut pcre2_match_data_8);
    pub fn pcre2_get_ovector_pointer_8(match_data: *mut pcre2_match_data_8) -> *mut PCRE2_SIZE;
    pub fn pcre2_get_mark_8(match_data: *mut pcre2_match_data_8) -> PCRE2_SPTR;

    pub fn pcre2_match_8(
        code: *const pcre2_code_8,
        subject: PCRE2_SPTR,
        length: PCRE2_SIZE,
        startoffset: PCRE2_SIZE,
        options: u32,
        match_data: *mut pcre2_match_data_8,
        mcontext: *mut pcre2_match_context_8,
    ) -> c_int;
    pub fn pcre2_jit_match_8(
        code: *const pcre2_code_8,
        subject: PCRE2_SPTR,
        length: PCRE2_SIZE,
        startoffset: PCRE2_SIZE,
        options: u32,
        match_data: *mut pcre2_match_data_8,
        mcontext: *mut pcre2_match_context_8,
    ) -> c_int;
}

// Compile options.
pub const PCRE2_ANCHORED: u32 = 0x8000_0000;
pub const PCRE2_NO_UTF_CHECK: u32 = 0x4000_0000;
pub const PCRE2_CASELESS: u32 = 0x0000_0008;
pub const PCRE2_DOLLAR_ENDONLY: u32 = 0x0000_0010;
pub const PCRE2_DOTALL: u32 = 0x0000_0020;
pub const PCRE2_DUPNAMES: u32 = 0x0000_0040;
pub const PCRE2_EXTENDED: u32 = 0x0000_0080;
pub const PCRE2_MULTILINE: u32 = 0x0000_0400;
pub const PCRE2_NO_AUTO_CAPTURE: u32 = 0x0000_2000;
pub const PCRE2_UCP: u32 = 0x0002_0000;
pub const PCRE2_UNGREEDY: u32 = 0x0004_0000;
pub const PCRE2_UTF: u32 = 0x0008_0000;
pub const PCRE2_NEVER_BACKSLASH_C: u32 = 0x0010_0000;

// Extra compile options (since 10.43).
pub const PCRE2_EXTRA_CASELESS_RESTRICT: u32 = 0x0000_0080;

// pcre2_jit_compile options.
pub const PCRE2_JIT_COMPLETE: u32 = 0x0000_0001;

// Match options.
pub const PCRE2_NOTEMPTY_ATSTART: u32 = 0x0000_0008;

// Error codes.
pub const PCRE2_ERROR_BACKSLASH_C_CALLER_DISABLED: c_int = 183;
pub const PCRE2_ERROR_NOMATCH: c_int = -1;
pub const PCRE2_ERROR_UTF8_ERR1: c_int = -3;
pub const PCRE2_ERROR_UTF8_ERR21: c_int = -23;
pub const PCRE2_ERROR_BADOFFSET: c_int = -33;
pub const PCRE2_ERROR_BADUTFOFFSET: c_int = -36;
pub const PCRE2_ERROR_INTERNAL: c_int = -44;
pub const PCRE2_ERROR_JIT_STACKLIMIT: c_int = -46;
pub const PCRE2_ERROR_MATCHLIMIT: c_int = -47;
pub const PCRE2_ERROR_NOMEMORY: c_int = -48;
/// Also `PCRE2_ERROR_RECURSIONLIMIT`.
pub const PCRE2_ERROR_DEPTHLIMIT: c_int = -53;

// pcre2_pattern_info requests.
pub const PCRE2_INFO_CAPTURECOUNT: u32 = 4;
pub const PCRE2_INFO_JITSIZE: u32 = 10;
pub const PCRE2_INFO_NAMECOUNT: u32 = 17;
pub const PCRE2_INFO_NAMEENTRYSIZE: u32 = 18;
pub const PCRE2_INFO_NAMETABLE: u32 = 19;

// pcre2_config requests.
pub const PCRE2_CONFIG_JIT: u32 = 1;
pub const PCRE2_CONFIG_JITTARGET: u32 = 2;
pub const PCRE2_CONFIG_UNICODE_VERSION: u32 = 10;
pub const PCRE2_CONFIG_VERSION: u32 = 11;

pub const PCRE2_UNSET: PCRE2_SIZE = !0;
