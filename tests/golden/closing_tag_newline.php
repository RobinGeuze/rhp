<?php
// A closing tag swallows the one newline right after it (\n or \r\n),
// nothing more: templates rely on it.
echo "a";
?>
b
<?php echo "c"; ?>

d
<?php if (true): ?>
  in-if
<?php endif; ?>
<?php foreach ([1, 2] as $i) { ?>
item <?= $i ?>
<?php } ?>
<?php function tpl($x) { ?>
fn <?= $x ?>
<?php }
tpl("x");
echo "e"; ?>
spaces-kept
<?php echo "f"; ?>
<?php echo "g";
echo "\n";
?>
