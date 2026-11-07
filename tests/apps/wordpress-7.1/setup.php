<?php
/**
 * rhp fixture: install WordPress non-interactively (`apps.py setup` runs
 * this with php after downloading core, the SQLite plugin and wp-cli).
 *
 * 1. Puts the SQLite Database Integration drop-in in place: the plugin's
 *    db.copy as wordpress/wp-content/db.php, as its readme says to.
 * 2. Creates a fresh database with WordPress's own wp_install(): fixed title,
 *    admin user, password and e-mail; no mail is sent.
 * 3. Seeds it reproducibly: pretty permalinks (rewrite rules stored, so no
 *    request writes them), the plugin activated, and every post, comment and
 *    user given the same fixed date, so the output never shows when setup ran.
 */

// This runs in the global scope WordPress loads into: its own variables are
// prefixed so they never clobber a WordPress global ($wp, ...).
if ( PHP_SAPI !== 'cli' ) {
	exit( 1 );
}

$rhp_root = __DIR__;
$rhp_wp = "$rhp_root/wordpress";
$rhp_plugin = "$rhp_wp/wp-content/plugins/sqlite-database-integration";
if ( ! is_file( "$rhp_plugin/db.copy" ) ) {
	fwrite( STDERR, "setup.php: run `python3 tests/apps/apps.py setup wordpress-7.1` (downloads WordPress)\n" );
	exit( 1 );
}

// 1. The drop-in.
copy( "$rhp_plugin/db.copy", "$rhp_wp/wp-content/db.php" );

// 2. A fresh database.
$rhp_db_dir = "$rhp_root/var/database";
foreach ( glob( "$rhp_db_dir/*" ) ?: array() as $rhp_f ) {
	unlink( $rhp_f );
}
@mkdir( $rhp_db_dir, 0777, true );

$_SERVER['HTTP_HOST'] = 'localhost';
$_SERVER['REQUEST_URI'] = '/';
$_SERVER['SERVER_PROTOCOL'] = 'HTTP/1.1';
define( 'WP_INSTALLING', true );
require "$rhp_wp/wp-load.php";

// wp_install() mails the admin; this pluggable function is defined first.
function wp_new_blog_notification( $blog_title, $blog_url, $user_id, $password ) {}
require_once ABSPATH . 'wp-admin/includes/upgrade.php';
require_once ABSPATH . 'wp-admin/includes/plugin.php';

$rhp_result = wp_install( 'rhp fixture', 'admin', 'admin@example.com', true, '', 'rhp-fixture-password', 'en_US' );
if ( is_wp_error( $rhp_result ) ) {
	fwrite( STDERR, 'wp_install: ' . $rhp_result->get_error_message() . "\n" );
	exit( 1 );
}

// 3. The seed.
update_option( 'blogdescription', 'A WordPress site rhp is measured against' );
update_option( 'timezone_string', 'UTC' );
update_option( 'permalink_structure', '/%postname%/' );
$wp_rewrite->init();
$wp_rewrite->flush_rules( false );

$rhp_result = activate_plugin( 'sqlite-database-integration/load.php' );
if ( is_wp_error( $rhp_result ) ) {
	fwrite( STDERR, 'activate_plugin: ' . $rhp_result->get_error_message() . "\n" );
	exit( 1 );
}

$rhp_when = '2026-01-01 12:00:00';
$wpdb->query( $wpdb->prepare( "UPDATE {$wpdb->posts} SET post_date = %s, post_date_gmt = %s, post_modified = %s, post_modified_gmt = %s", $rhp_when, $rhp_when, $rhp_when, $rhp_when ) );
$wpdb->query( $wpdb->prepare( "UPDATE {$wpdb->comments} SET comment_date = %s, comment_date_gmt = %s", $rhp_when, $rhp_when ) );
$wpdb->query( $wpdb->prepare( "UPDATE {$wpdb->users} SET user_registered = %s", $rhp_when ) );
wp_cache_flush();

echo "WordPress {$wp_version} installed: http://localhost/ (admin / rhp-fixture-password)\n";
