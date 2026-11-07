<?php
query_posts("p=1"); ob_start(); load_template(ABSPATH . WPINC . '/feed-rss2.php'); $x = ob_get_clean(); echo preg_replace('/<lastBuildDate>.*?</', '<', $x);
query_posts(''); ob_start(); load_template(ABSPATH . WPINC . '/feed-atom.php'); echo preg_replace('/<updated>.*?</', '<', ob_get_clean());
