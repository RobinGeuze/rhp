<?php
/**
 * rhp fixture, test only: a request with an `X-Rhp-User: <id>` header is
 * logged in as that user, without a session token, so admin pages and their
 * nonces are the same for php and rhp (within WordPress's 12-hour nonce
 * tick). `setup.php` copies this into wp-content/mu-plugins/. Without the
 * header WordPress's own wp_validate_auth_cookie() is used.
 */
if ( isset( $_SERVER['HTTP_X_RHP_USER'] ) && ! function_exists( 'wp_validate_auth_cookie' ) ) {
	function wp_validate_auth_cookie( $cookie = '', $scheme = '' ) {
		return (int) $_SERVER['HTTP_X_RHP_USER'];
	}
}
