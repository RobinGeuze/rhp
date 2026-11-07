<?php
$x = new WP_HTML_Tag_Processor('<div class="a b" data-x="1"><img src="x.jpg" alt="&amp;"></div>'); while ($x->next_tag()) { $x->add_class('z'); $x->set_attribute('data-y', '"q"'); } echo $x->get_updated_html(), "\n";
$p = WP_HTML_Processor::create_fragment('<p>One<p>Two<table><tr><td>x</table><ul><li>a<li>b</ul>'); $o = ''; while ($p->next_token()) $o .= $p->get_token_name() . ','; echo $o, "\n";
echo wp_json_encode(wp_parse_args('a=1&b[]=2', array('c' => 3))), wp_json_encode(wp_list_pluck(get_posts(array('post_type' => 'any')), 'post_title')), "\n";
echo wp_json_encode(get_option('rewrite_rules') ? count(get_option('rewrite_rules')) : 0), url_to_postid('http://localhost/hello-world/'), "\n";
$q = new WP_Query(array('s' => 'hello', 'orderby' => 'relevance')); echo $q->request, "\n", $q->found_posts, "\n";
$q = new WP_Query(array('meta_query' => array(array('key' => '_wp_page_template', 'compare' => 'NOT EXISTS')), 'tax_query' => array(array('taxonomy' => 'category', 'field' => 'slug', 'terms' => 'uncategorized')), 'date_query' => array('after' => '2025-01-01'))); echo $q->post_count, "\n";
echo wp_json_encode(wp_count_posts()), wp_json_encode(wp_count_comments()), wp_json_encode(count_users()), "\n";
echo get_avatar(1), "\n", get_permalink(1), get_edit_post_link(1, 'raw'), "\n";
echo wp_json_encode(wp_get_theme()->get('Name')), wp_json_encode(array_keys(wp_get_themes())), "\n";
echo wp_json_encode(get_plugins()), "\n";
echo wp_json_encode(WP_Block_Type_Registry::get_instance()->is_registered('core/paragraph')), count(WP_Block_Type_Registry::get_instance()->get_all_registered()), "\n";
echo wp_json_encode(wp_get_image_editor(ABSPATH . 'wp-admin/images/w-logo-blue.png') instanceof WP_Error), wp_json_encode(wp_image_editor_supports(array('mime_type' => 'image/webp'))), "\n";
echo wp_json_encode(getimagesize(ABSPATH . 'wp-admin/images/w-logo-blue.png')), wp_json_encode(wp_check_filetype_and_ext(ABSPATH . 'wp-admin/images/w-logo-blue.png', 'w.png')), "\n";
echo wp_json_encode(wp_get_mime_types()['jpg|jpeg|jpe']), wp_json_encode(function_exists('exif_read_data')), wp_json_encode(function_exists('imagecreatefromstring')), wp_json_encode(class_exists('Imagick')), wp_json_encode(function_exists('finfo_open')), wp_json_encode(function_exists('mysqli_connect')), wp_json_encode(function_exists('sodium_crypto_box')), "\n";
echo wp_json_encode(apply_filters('the_content', 'Hello <b>x</b> https://www.youtube.com/watch?v=x')), "\n";
