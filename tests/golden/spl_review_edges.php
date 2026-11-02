<?php
// SPL edge cases against php: SplFixedArray's index conversion and errors,
// debug views (var_dump/print_r) of the SPL structures, SplObjectStorage's
// cursor before rewind(), LimitIterator::seek below the offset,
// MultipleIterator's lazy MIT_KEYS_ASSOC check, setInfoClass validation.

$f = SplFixedArray::fromArray([1, null, 'x']);
$h = new SplMinHeap; $h->insert(3); $h->insert(1);
$q = new SplPriorityQueue; $q->insert('a', 2);
$st = new SplStack; $st->push(1);
$dl = new SplDoublyLinkedList; $dl->setIteratorMode(SplDoublyLinkedList::IT_MODE_DELETE); $dl->push('d');
$os = new SplObjectStorage; $os[$f] = 'inf';
$fi = new SplFileInfo('dir/name.txt');
$hc = hash_init('sha1');
var_dump($f, $h, $q, $st, $dl, $os, $fi, $hc);
print_r($f); print_r($h); echo "\n";
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });

$f = new SplFixedArray(3);
$f[0] = 'a'; $f[2] = 0;
foreach (['1', 1.5, 'x', '01', -1, 3, null, true, false, [], 1e30, '-1'] as $i) {
    echo json_encode($i), ': ';
    try { echo json_encode(isset($f[$i])), ' '; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), ' | '; }
    try { echo json_encode(empty($f[$i])), ' '; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), ' | '; }
    try { echo json_encode($f[$i]), ' '; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), ' | '; }
    try { echo json_encode($f[$i] ?? 'dflt'), ' '; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), ' | '; }
    try { unset($f[$i]); echo 'u '; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), ' | '; }
    echo "\n";
}
try { $f[] = 1; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$f = new SplFixedArray(3); $f[0] = 'a';
var_dump($f->setSize(5), $f->toArray(), $f->setSize(1), $f->toArray());
try { $f->setSize(-1); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { new SplFixedArray(-1); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { new SplFixedArray('x'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(SplFixedArray::fromArray([2 => 'a', 0 => 'b'])->toArray(), SplFixedArray::fromArray([2 => 'a', 'k' => 'b'], false)->toArray());
try { SplFixedArray::fromArray(['x' => 1]); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$f = SplFixedArray::fromArray([1, 2, 3]);
var_dump(count($f), json_encode($f), $f instanceof IteratorAggregate, $f instanceof Iterator);
foreach ($f as $k => $v) { echo "$k=$v "; foreach ($f as $k2 => $v2) echo "[$k2]"; } echo "\n";

$s = new SplObjectStorage; $o = new stdClass; $s[$o] = 1; $s[new stdClass] = 2;
var_dump($s->valid(), $s->key(), $s->current() === $o, $s->getInfo());
$s->next(); var_dump($s->key(), $s->getInfo()); $s->seek(0); var_dump($s->getInfo());

$l = new LimitIterator(new ArrayIterator(['a' => 1, 'b' => 2, 'c' => 3]), 1, 2);
try { $l->seek(0); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }
$l->seek(2); var_dump($l->key());

$m = new MultipleIterator(MultipleIterator::MIT_KEYS_ASSOC);
$m->attachIterator(new ArrayIterator([1]));
echo "attached\n";
try { foreach ($m as $v) {} } catch (InvalidArgumentException $e) { echo $e->getMessage(), "\n"; }

$fi = new SplFileInfo('x');
try { $fi->setInfoClass('stdClass'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $fi->setFileClass('SplFileInfo'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
