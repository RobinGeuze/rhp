<?php
// Built-in classes list their methods in their stub's order.
foreach (['LimitIterator', 'SplHeap', 'SplObjectStorage', 'ArrayObject', 'SplFixedArray', 'ReflectionReference'] as $c) {
    echo $c, ': ', implode(',', get_class_methods($c)), "\n";
}
echo implode(',', array_map(fn($m) => $m->class . '::' . $m->name, (new ReflectionClass('CachingIterator'))->getMethods())), "\n";
