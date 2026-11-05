//! Zend/zend_constants.c: the engine's standard constants
//! (`zend_register_standard_constants`), and zend_execute's
//! `ZEND_VM_KIND`.

use rhp_values::Value;

use crate::{
    version, E_ALL, E_COMPILE_ERROR, E_COMPILE_WARNING, E_CORE_ERROR, E_CORE_WARNING, E_DEPRECATED,
    E_ERROR, E_NOTICE, E_PARSE, E_RECOVERABLE_ERROR, E_STRICT, E_USER_DEPRECATED, E_USER_ERROR,
    E_USER_NOTICE, E_USER_WARNING, E_WARNING,
};

pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("E_ERROR", Value::Int(E_ERROR)),
        ("E_WARNING", Value::Int(E_WARNING)),
        ("E_PARSE", Value::Int(E_PARSE)),
        ("E_NOTICE", Value::Int(E_NOTICE)),
        ("E_CORE_ERROR", Value::Int(E_CORE_ERROR)),
        ("E_CORE_WARNING", Value::Int(E_CORE_WARNING)),
        ("E_COMPILE_ERROR", Value::Int(E_COMPILE_ERROR)),
        ("E_COMPILE_WARNING", Value::Int(E_COMPILE_WARNING)),
        ("E_USER_ERROR", Value::Int(E_USER_ERROR)),
        ("E_USER_WARNING", Value::Int(E_USER_WARNING)),
        ("E_USER_NOTICE", Value::Int(E_USER_NOTICE)),
        ("E_STRICT", Value::Int(E_STRICT)),
        ("E_RECOVERABLE_ERROR", Value::Int(E_RECOVERABLE_ERROR)),
        ("E_DEPRECATED", Value::Int(E_DEPRECATED)),
        ("E_USER_DEPRECATED", Value::Int(E_USER_DEPRECATED)),
        ("E_ALL", Value::Int(E_ALL)),
        ("DEBUG_BACKTRACE_PROVIDE_OBJECT", Value::Int(1)),
        ("DEBUG_BACKTRACE_IGNORE_ARGS", Value::Int(2)),
        ("ZEND_THREAD_SAFE", Value::Bool(false)),
        ("ZEND_DEBUG_BUILD", Value::Bool(false)),
        ("TRUE", Value::Bool(true)),
        ("FALSE", Value::Bool(false)),
        ("NULL", Value::Null),
        ("ZEND_VM_KIND", Value::str(version::ZEND_VM_KIND)),
    ]
}
