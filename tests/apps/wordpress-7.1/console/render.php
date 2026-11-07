<?php
echo do_blocks(get_post(1)->post_content), do_blocks(get_post(2)->post_content);
echo do_blocks('<!-- wp:group {"layout":{"type":"constrained"}} --><div class="wp-block-group"><!-- wp:heading --><h2>H</h2><!-- /wp:heading --><!-- wp:columns --><div class="wp-block-columns"><!-- wp:column --><div class="wp-block-column"><!-- wp:latest-posts /--></div><!-- /wp:column --></div><!-- /wp:columns --></div><!-- /wp:group -->');
echo wp_json_encode(parse_blocks(get_post(2)->post_content));
echo strlen(wp_get_global_stylesheet()), "\n", md5(wp_get_global_stylesheet()), "\n";
echo md5(wp_json_encode(wp_get_global_settings())), "\n";
echo get_the_block_template_html() ?? '', "\n";
