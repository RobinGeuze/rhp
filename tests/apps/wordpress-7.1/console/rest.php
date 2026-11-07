<?php
wp_set_current_user(1);
foreach (array('/', '/wp/v2/posts', '/wp/v2/posts/1', '/wp/v2/pages', '/wp/v2/users', '/wp/v2/types', '/wp/v2/taxonomies', '/wp/v2/settings', '/wp/v2/block-types', '/wp/v2/templates', '/wp/v2/global-styles/themes/twentytwentyfive', '/wp/v2/themes', '/wp/v2/plugins', '/wp/v2/search?search=hello', '/wp/v2/comments', '/wp/v2/block-patterns/patterns', '/wp/v2/navigation', '/wp-site-health/v1/tests/background-updates') as $route) {
  $u = wp_parse_url($route); $r = new WP_REST_Request('GET', $u['path']); if (isset($u['query'])) { parse_str($u['query'], $q); $r->set_query_params($q); }
  $res = rest_do_request($r); $j = wp_json_encode($res->get_data()); echo $route, ' ', $res->get_status(), ' ', strlen($j), ' ', md5($j), "\n";
}
$r = new WP_REST_Request('POST', '/wp/v2/posts'); $r->set_body_params(array('title' => 'REST', 'content' => '<!-- wp:paragraph --><p>x</p><!-- /wp:paragraph -->', 'status' => 'draft', 'date' => '2026-01-03T00:00:00'));
$res = rest_do_request($r); echo $res->get_status(), ' ', $res->get_data()['slug'] ?? '', "\n";
