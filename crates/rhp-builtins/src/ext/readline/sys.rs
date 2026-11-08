//! libedit's readline compatibility interface (`<editline/readline.h>`),
//! linked as php links it: the macOS SDK's /usr/lib/libedit and the
//! distro's libedit on Linux, both dynamically (docs/design/libraries.md).

use std::ffi::{c_char, c_int};

/// `rl_compentry_func_t`.
pub type CompEntry = unsafe extern "C" fn(text: *const c_char, state: c_int) -> *mut c_char;
/// `CPPFunction`: `rl_attempted_completion_function`.
pub type AttemptedCompletion =
    unsafe extern "C" fn(text: *const c_char, start: c_int, end: c_int) -> *mut *mut c_char;
/// `VCPFunction`: the callback interface's line handler.
pub type LineHandler = unsafe extern "C" fn(line: *mut c_char);

#[link(name = "edit", kind = "dylib")]
extern "C" {
    pub fn readline(prompt: *const c_char) -> *mut c_char;
    pub fn using_history();
    pub fn add_history(line: *const c_char) -> c_int;
    pub fn clear_history();
    pub fn read_history(file: *const c_char) -> c_int;
    pub fn write_history(file: *const c_char) -> c_int;
    pub fn rl_completion_matches(text: *const c_char, gen: CompEntry) -> *mut *mut c_char;
    pub fn rl_callback_handler_install(prompt: *const c_char, handler: LineHandler);
    pub fn rl_callback_read_char();
    pub fn rl_callback_handler_remove();
    pub fn rl_redisplay();
    pub fn rl_on_new_line() -> c_int;

    pub static mut rl_line_buffer: *mut c_char;
    pub static mut rl_point: c_int;
    pub static mut rl_end: c_int;
    pub static rl_library_version: *const c_char;
    pub static mut rl_readline_name: *const c_char;
    pub static mut rl_attempted_completion_over: c_int;
    pub static mut rl_attempted_completion_function: Option<AttemptedCompletion>;
}

#[cfg(test)]
mod tests;
