<?php
echo wpautop("a\n\nb\nc<div>x</div>\n\nd"), wptexturize('"quotes" -- ... it\'s (c) 1x2'), "\n";
echo sanitize_title('Héllo Wörld! Ça va?'), '|', remove_accents('ÀÉÎÕÜ ß ø'), '|', number_format_i18n(1234567.891, 2), '|', size_format(123456789), "\n";
echo wp_trim_words(str_repeat('héllo wörld ', 50), 10), '|', mb_strimwidth('héllo wörld', 0, 6, '…'), "\n";
echo seems_utf8("\xC3\xA9") ? 'y' : 'n', wp_check_invalid_utf8("a\xFFb", true), '|', utf8_uri_encode('héllo'), '|', urlencode_deep('a b'), "\n";
echo zeroise(7, 3), '|', ent2ncr('&hellip;&copy;'), '|', wp_specialchars_decode('&lt;&#039;'), '|', capital_P_dangit('Wordpress'), "\n";
echo human_time_diff(0, 3600), '|', human_time_diff(0, 86400 * 40), "\n";
echo date_i18n('l jS F Y H:i', 1767268800), '|', wp_date('c D M', 1767268800), '|', mysql2date('U', '2026-01-01 12:00:00'), "\n";
echo wp_json_encode(array('a' => 1.5, 'b' => "é\u{1F600}", 'c' => null, 'd' => array()), JSON_PRETTY_PRINT), "\n";
var_dump(maybe_unserialize(serialize(array(1, 'x' => new stdClass))), is_serialized('a:0:{}'));
echo wp_hash('x'), '|', wp_create_nonce('a'), '|', wp_hash_password('pw') !== '' ? 'h' : '', '|', wp_check_password('rhp-fixture-password', get_userdata(1)->user_pass) ? 'ok' : 'bad', "\n";
echo md5(serialize(wp_parse_url('http://u:p@h:8/p?q#f'))), '|', add_query_arg(array('a' => 'b c', 'd' => false), 'http://x/?e=1'), "\n";
echo shortcode_parse_atts('a="1" b=2 c'), do_shortcode('[caption id="x" width="300"]<img src="a.jpg"> Cap[/caption][gallery ids="1"]'), "\n";
printf("%s\n", get_the_excerpt(1));
