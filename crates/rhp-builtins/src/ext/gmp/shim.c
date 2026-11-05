/* Checks ext/gmp/sys.rs against the gmp.h build.rs links: the layouts it
 * mirrors, and the types it assumes. */

#include <limits.h>
#include <stddef.h>
#include <gmp.h>

_Static_assert(GMP_LIMB_BITS == 64 && GMP_NAIL_BITS == 0,
               "sys.rs: mp_limb_t is a 64-bit limb without nails");
_Static_assert(sizeof(mp_limb_t) == 8, "sys.rs: mp_limb_t = u64");
_Static_assert(sizeof(mp_bitcnt_t) == sizeof(unsigned long), "sys.rs: mp_bitcnt_t = c_ulong");
_Static_assert(sizeof(__mpz_struct) == 16 && offsetof(__mpz_struct, _mp_alloc) == 0 &&
                   offsetof(__mpz_struct, _mp_size) == 4 && offsetof(__mpz_struct, _mp_d) == 8,
               "sys.rs: MpzStruct mirrors __mpz_struct");
_Static_assert(sizeof(__gmp_randstate_struct) == 32 && _Alignof(__gmp_randstate_struct) <= 8,
               "sys.rs: RandStruct is 32 bytes, 8-aligned");
_Static_assert(INT_MAX == 2147483647, "sys.rs: MAX_LIMBS assumes a 32-bit int");

#define STR2(x) #x
#define STR(x) STR2(x)

const char *rhp_gmp_header_version(void) {
    return STR(__GNU_MP_VERSION) "." STR(__GNU_MP_VERSION_MINOR) "." STR(__GNU_MP_VERSION_PATCHLEVEL);
}
