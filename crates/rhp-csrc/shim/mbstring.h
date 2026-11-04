/*
 * Stand-in for ext/mbstring/mbstring.h, which libmbfl's mbfilter.c and
 * php_unicode.c include. They use only libmbfl's own headers and the
 * export macro from it, never the module globals.
 */
#ifndef RHP_SHIM_MBSTRING_H
#define RHP_SHIM_MBSTRING_H

#include "zend_shim.h"

#define MBSTRING_API
#define ZEND_EXTERN_MODULE_GLOBALS(module_name)

#include "libmbfl/mbfl/mbfilter.h"

#endif
