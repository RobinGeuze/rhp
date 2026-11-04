//! main/output.c: the userland output-control functions (`ob_*`). The
//! handler stack itself lives in the host ([`crate::Host::output_control`]).

use rhp_values::ops::deref;
use rhp_values::Value;

use crate::{Host, Thrown};

pub(crate) fn ob(host: &mut dyn Host, call: crate::ObCall) -> Result<Value, Thrown> {
    host.output_control(call)
}

fn fn_ob_start(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let callback = args.first().map(deref).unwrap_or(Value::Null);
    let chunk_size = crate::zend::zpp::int_param(host, args, 1, "ob_start", "chunk_size", 0)?;
    let flags = crate::zend::zpp::int_param(host, args, 2, "ob_start", "flags", 0x70)?;
    ob(
        host,
        crate::ObCall::Start {
            callback,
            chunk_size,
            flags,
        },
    )
}

fn fn_ob_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::Flush)
}

fn fn_ob_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::Clean)
}

fn fn_ob_end_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::EndFlush)
}

fn fn_ob_end_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::EndClean)
}

fn fn_ob_get_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetFlush)
}

fn fn_ob_get_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetClean)
}

fn fn_ob_get_contents(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetContents)
}

fn fn_ob_get_length(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetLength)
}

fn fn_ob_get_level(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetLevel)
}

fn fn_ob_list_handlers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::ListHandlers)
}

fn fn_ob_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let full = crate::zend::zpp::bool_param(host, args, 0, "ob_get_status", "full_status", false)?;
    ob(host, crate::ObCall::GetStatus { full })
}

fn fn_ob_implicit_flush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let on = crate::zend::zpp::bool_param(host, args, 0, "ob_implicit_flush", "enable", true)?;
    ob(host, crate::ObCall::ImplicitFlush(on))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("ob_start", fn_ob_start),
            ("ob_flush", fn_ob_flush),
            ("ob_clean", fn_ob_clean),
            ("ob_end_flush", fn_ob_end_flush),
            ("ob_end_clean", fn_ob_end_clean),
            ("ob_get_flush", fn_ob_get_flush),
            ("ob_get_clean", fn_ob_get_clean),
            ("ob_get_contents", fn_ob_get_contents),
            ("ob_get_length", fn_ob_get_length),
            ("ob_get_level", fn_ob_get_level),
            ("ob_list_handlers", fn_ob_list_handlers),
            ("ob_get_status", fn_ob_get_status),
            ("ob_implicit_flush", fn_ob_implicit_flush),
        ],
    );
}
