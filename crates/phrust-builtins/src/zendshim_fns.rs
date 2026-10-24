//! Builtins backed by PHP extension C compiled against the Zend macro shim.
//!
//! Feature-gated (`zendshim`, off by default). These exercise the
//! `phrust-zendshim` bridge end to end; they are named with a `__zendshim_`
//! prefix so they never shadow the pure-Rust builtins and can be diffed against
//! them (and against php) in tests.

use std::collections::HashMap;

use phrust_values::Value;

use crate::{Builtin, Host, Thrown};

fn fn_demo(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    phrust_zendshim::demo_sum_and_label(args)
}

fn fn_bin2hex(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    phrust_zendshim::bin2hex(args)
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__zendshim_demo",
        Builtin { name: "__zendshim_demo", func: fn_demo, min_args: 3, max_args: Some(3) },
    );
    m.insert(
        "__zendshim_bin2hex",
        Builtin { name: "__zendshim_bin2hex", func: fn_bin2hex, min_args: 1, max_args: Some(1) },
    );
}
