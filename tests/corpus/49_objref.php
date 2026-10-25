<?php
// by-reference foreach over an object's properties + dynamic property lvalue,
// plus recursion that mutates nested objects through the shared handle.

function expand(&$node, $prefix) {
    if (!is_object($node)) {
        return;
    }
    if (property_exists($node, 'ref') && is_string($node->ref)) {
        $node->ref = $prefix . $node->ref;
    }
    foreach ($node as $key => &$member) {
        expand($member, $prefix);
    }
    unset($member);
}

$tree = json_decode('{
    "ref": "a",
    "kids": {
        "x": {"ref": "b"},
        "y": {"items": {"ref": "c"}}
    }
}');
expand($tree, "EXP:");
echo json_encode($tree), "\n";

// Dynamic property as an assignment target (stdClass allows dynamic props).
$b = new stdClass();
$b->data = [];
$fields = ['alpha', 'beta', 'gamma'];
foreach ($fields as $i => $name) {
    $b->$name = ($i + 1) * 10;
}
$b->{'de' . 'lta'} = 99;
echo $b->alpha, " ", $b->beta, " ", $b->gamma, " ", $b->delta, "\n";

// Nested dynamic property + array element lvalue.
$key = 'data';
$b->$key['first'] = 'hello';
echo $b->data['first'], "\n";

// by-ref foreach that increments every numeric property in place.
$counts = new stdClass();
$counts->a = 1;
$counts->b = 2;
$counts->c = 3;
foreach ($counts as $k => &$v) {
    $v *= 100;
}
unset($v);
echo $counts->a, " ", $counts->b, " ", $counts->c, "\n";
