/* argon2.h's parameter limits, as values Rust can read. */
#include "argon2.h"

const uint64_t rhp_argon2_min_memory = ARGON2_MIN_MEMORY;
const uint64_t rhp_argon2_max_memory = ARGON2_MAX_MEMORY;
const uint64_t rhp_argon2_min_time = ARGON2_MIN_TIME;
const uint64_t rhp_argon2_max_time = ARGON2_MAX_TIME;
const uint64_t rhp_argon2_max_lanes = ARGON2_MAX_LANES;
const uint32_t rhp_argon2_version_number = ARGON2_VERSION_NUMBER;

_Static_assert(sizeof(argon2_type) == sizeof(int), "argon2_type is passed as a C int");
_Static_assert(Argon2_i == 1 && Argon2_id == 2, "argon2_type values");
