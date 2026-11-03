/* The DES entry point for Rust, keeping php_crypt_extended_data on the C side. */
#include <string.h>

#include "crypt_freesec.h"

_Static_assert(sizeof(((struct php_crypt_extended_data *)0)->output) == 21,
	"rhp_crypt_des copies at most 21 bytes");

/*
 * php_crypt()'s DES branch with fresh per-call state, so concurrent calls are
 * safe. _crypt_extended_init() must have run once before (crypt.rs does it).
 * Writes the NUL-terminated hash (at most 20 characters) to out.
 */
int rhp_crypt_des(const char *key, const char *setting, char out[21])
{
	struct php_crypt_extended_data data;
	char *res;

	memset(&data, 0, sizeof(data));
	res = _crypt_extended_r((const unsigned char *)key, setting, &data);
	if (!res) {
		return 0;
	}
	memcpy(out, res, strlen(res) + 1);
	return 1;
}
