<?php
// An ArrayObject / ArrayIterator over another one uses that one's storage.
$in = new ArrayObject([1, 2, 3]);
$ao = @new ArrayObject($in);
var_dump(count($ao), $ao[1]);
foreach ($ao as $k => $v) echo "$k=$v "; echo "\n";
$ao[] = 4; var_dump(count($in));
$in['x'] = 'y'; var_dump($ao['x']);
$ai = new ArrayIterator(['a' => 1]);
$ao2 = @new ArrayObject($ai); $ao2['b'] = 2;
var_dump($ao2->getArrayCopy(), iterator_to_array($ai));
$it = @new ArrayIterator($in); var_dump(count($it), $it->current());
$old = $ao->exchangeArray([9]); var_dump(count($old), count($in), $ao->getArrayCopy());
var_dump($ao2);
var_dump(unserialize(serialize($ao2))->getArrayCopy());
