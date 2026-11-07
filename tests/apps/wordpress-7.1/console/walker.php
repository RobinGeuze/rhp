<?php
echo wp_dropdown_categories(array('echo' => 0, 'hide_empty' => 0)), "\n";
echo wp_dropdown_pages(array('echo' => 0)), "\n";
var_dump(count(get_terms(array('taxonomy' => 'category', 'hide_empty' => false))));
$t = get_terms(array('taxonomy' => 'category', 'hide_empty' => false)); $w = new Walker_CategoryDropdown(); echo $w->walk($t, 0, array('show_count' => 0, 'selected' => 0, 'value_field' => 'term_id')), "\n";
$e = array((object) array('term_id' => 1, 'parent' => 0, 'name' => 'A'), (object) array('term_id' => 2, 'parent' => 1, 'name' => 'B'));
class W extends Walker { public $db_fields = array('parent' => 'parent', 'id' => 'term_id'); public function start_el(&$o, $d, $depth = 0, $a = array(), $id = 0) { $o .= "[$depth:{$d->name}]"; } }
$w = new W(); echo $w->walk($e, 0), '|', $w->walk($e, -1), '|', $w->walk(array($e[0]), 0), "\n";
