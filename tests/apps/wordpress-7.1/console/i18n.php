<?php
echo __('Hello world!'), _n('%s post', '%s posts', 2), _x('Draft', 'post status'), "\n";
load_textdomain('x', '/nonexistent.mo'); echo translate('Save', 'x'), "\n";
require_once ABSPATH . WPINC . '/pomo/po.php'; $po = new PO(); $po->add_entry(new Translation_Entry(array('singular' => 'a', 'translations' => array('b')))); $mo = new MO(); $mo->entries = $po->entries; $f = tempnam(sys_get_temp_dir(), 'mo'); $mo->export_to_file($f); echo md5_file($f), "\n";
$m = new MO(); $m->import_from_file($f); echo $m->translate('a'), "\n";
echo wp_json_encode(WP_Translation_Controller::get_instance()->load_file($f, 'y')), translate('a', 'y'), "\n";
$p = new Plural_Forms('(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2)'); echo $p->get(22), $p->get(11), "\n";
