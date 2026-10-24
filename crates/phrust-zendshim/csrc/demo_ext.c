/* demo_ext.c — Stage 1 proof: a hand-written "extension" function written with
 * the REAL Zend macro spellings, compiled against zend_shim.h (never linked to
 * php). Proves params-in, array building, strings, and value-out all flow
 * through the shim.
 *
 * This is verbatim the kind of C a PHP ext author writes. The only non-standard
 * line is `#include "zend_shim.h"` instead of `#include "php.h"`.
 */
#include "zend_shim.h"

PHP_FUNCTION(demo_sum_and_label) {
    zend_long a, b;
    char *label;
    size_t label_len;

    ZEND_PARSE_PARAMETERS_START(3, 3)
        Z_PARAM_LONG(a)
        Z_PARAM_LONG(b)
        Z_PARAM_STRING(label, label_len)
    ZEND_PARSE_PARAMETERS_END();

    array_init(return_value);
    add_assoc_long(return_value, "sum", a + b);
    add_assoc_string(return_value, "label", label);
    add_next_index_long(return_value, a);
}
