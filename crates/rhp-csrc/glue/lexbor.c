/*
 * What ext/lexbor's MINIT does: point lexbor's allocator at emalloc, so its
 * memory counts against memory_limit as in php (php_lexbor.c).
 */
#include "zend_shim.h"
#include "lexbor/core/base.h"
#include "lexbor/core/lexbor.h"

static void *glue_malloc(size_t size)
{
	return emalloc(size);
}

static void *glue_realloc(void *dst, size_t size)
{
	return erealloc(dst, size);
}

static void *glue_calloc(size_t num, size_t size)
{
	return ecalloc(num, size);
}

static void glue_free(void *ptr)
{
	efree(ptr);
}

void rhp_lexbor_setup(void)
{
	lexbor_memory_setup(glue_malloc, glue_realloc, glue_calloc, glue_free);
}
