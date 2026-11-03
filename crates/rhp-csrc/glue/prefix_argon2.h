/*
 * Force-included (-include) into every C file of the argon2 archive. Gives
 * each global symbol the reference argon2 C defines an rhp_ prefix. libsodium
 * exports argon2_ctx, argon2_hash, blake2b_init and others under the same
 * names with different struct layouts, and core.c's names (initialize,
 * finalize, ...) are generic. Macros only: this comes before every system
 * header. src/lib.rs tests that the archive exports nothing else.
 */
#ifndef RHP_PREFIX_ARGON2_H
#define RHP_PREFIX_ARGON2_H

/* argon2.c */
#define argon2_ctx rhp_argon2_ctx
#define argon2_encodedlen rhp_argon2_encodedlen
#define argon2_error_message rhp_argon2_error_message
#define argon2_hash rhp_argon2_hash
#define argon2_type2string rhp_argon2_type2string
#define argon2_verify rhp_argon2_verify
#define argon2_verify_ctx rhp_argon2_verify_ctx
#define argon2d_ctx rhp_argon2d_ctx
#define argon2d_hash_encoded rhp_argon2d_hash_encoded
#define argon2d_hash_raw rhp_argon2d_hash_raw
#define argon2d_verify rhp_argon2d_verify
#define argon2d_verify_ctx rhp_argon2d_verify_ctx
#define argon2i_ctx rhp_argon2i_ctx
#define argon2i_hash_encoded rhp_argon2i_hash_encoded
#define argon2i_hash_raw rhp_argon2i_hash_raw
#define argon2i_verify rhp_argon2i_verify
#define argon2i_verify_ctx rhp_argon2i_verify_ctx
#define argon2id_ctx rhp_argon2id_ctx
#define argon2id_hash_encoded rhp_argon2id_hash_encoded
#define argon2id_hash_raw rhp_argon2id_hash_raw
#define argon2id_verify rhp_argon2id_verify
#define argon2id_verify_ctx rhp_argon2id_verify_ctx

/* core.c, ref.c */
#define FLAG_clear_internal_memory rhp_argon2_FLAG_clear_internal_memory
#define allocate_memory rhp_argon2_allocate_memory
#define clear_internal_memory rhp_argon2_clear_internal_memory
#define copy_block rhp_argon2_copy_block
#define fill_first_blocks rhp_argon2_fill_first_blocks
#define fill_memory_blocks rhp_argon2_fill_memory_blocks
#define fill_segment rhp_argon2_fill_segment
#define finalize rhp_argon2_finalize
#define free_memory rhp_argon2_free_memory
#define index_alpha rhp_argon2_index_alpha
#define init_block_value rhp_argon2_init_block_value
#define initial_hash rhp_argon2_initial_hash
#define initialize rhp_argon2_initialize
#define secure_wipe_memory rhp_argon2_secure_wipe_memory
#define validate_inputs rhp_argon2_validate_inputs
#define xor_block rhp_argon2_xor_block

/* encoding.c */
#define b64len rhp_argon2_b64len
#define decode_string rhp_argon2_decode_string
#define encode_string rhp_argon2_encode_string
#define numlen rhp_argon2_numlen

/* thread.c */
#define argon2_thread_create rhp_argon2_thread_create
#define argon2_thread_exit rhp_argon2_thread_exit
#define argon2_thread_join rhp_argon2_thread_join

/* blake2/blake2b.c */
#define blake2b rhp_blake2b
#define blake2b_final rhp_blake2b_final
#define blake2b_init rhp_blake2b_init
#define blake2b_init_key rhp_blake2b_init_key
#define blake2b_init_param rhp_blake2b_init_param
#define blake2b_long rhp_blake2b_long
#define blake2b_update rhp_blake2b_update

#endif
