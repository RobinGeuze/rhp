/*
 * Force-included (-include) into sapi/cli's php_http_parser.c and the glue.
 * Gives each global symbol the parser defines an rhp_ prefix (see
 * prefix_crypt.h).
 */
#ifndef RHP_PREFIX_CLI_SERVER_H
#define RHP_PREFIX_CLI_SERVER_H

#define php_http_parser_init rhp_php_http_parser_init
#define php_http_parser_execute rhp_php_http_parser_execute
#define php_http_should_keep_alive rhp_php_http_should_keep_alive
#define php_http_method_str rhp_php_http_method_str

#endif
