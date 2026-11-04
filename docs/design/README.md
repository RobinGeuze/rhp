# Design docs

Start with the project [README](../../README.md), then
[`bytecode.md`](./bytecode.md). How rhp is tested is in
[`../testing.md`](../testing.md).

## Current

| doc | what it covers |
|---|---|
| [`bytecode.md`](./bytecode.md) | The compiler and the register VM: the ISA, `FunctionProto`, the register model, calls, exceptions. Its crate and migration sections record the plan as executed. |
| [`oop.md`](./oop.md) | Classes, objects and methods in the VM: linking, the class table, property and method dispatch, late static binding. |
| [`oop-model.md`](./oop-model.md) | The runtime class table and object types that `oop.md` builds on. |
| [`value-model.md`](./value-model.md) | `Value` (the zval analogue), `PhpStr`, `PhpArray`, references, copy-on-write. |
| [`php-semantics.md`](./php-semantics.md) | php's exact output, number, comparison and conversion rules, as probed on the reference php. |
| [`sapi.md`](./sapi.md) | The request layer (`rhp-sapi`): the `Sapi` trait, the request lifecycle, the compiled-script cache, memory accounting, embedding. |
| [`request-state.md`](./request-state.md) | Every `thread_local!`: what it holds, whether it is per request, per call or a per-thread cache, and what resets it. |
| [`libraries.md`](./libraries.md) | Policy for the C libraries rhp links or compiles: which, how (static or dynamic), licences, threading, the FFI callback layer. |
| [`extensions.md`](./extensions.md) | Research on reusing php's native code per extension. `libraries.md` replaces its linking policy. |
| [`placeholders.md`](./placeholders.md) | Backlog: where rhp still fakes, stubs or approximates php behaviour. |

## History

[`history/`](./history/) keeps superseded design documents for the record.
They describe plans as they were written, not the code as it is.

| doc | what it was |
|---|---|
| [`scaffold.md`](./history/scaffold.md) | The original contract for the first engine, a tree-walking interpreter. |
| [`reorg.md`](./history/reorg.md) | The crate reorganisation from the tree-walker to the bytecode VM (executed). |
| [`vm.md`](./history/vm.md) | The VM architecture as first planned, inside the tree-walker's crate. |
| [`isa.md`](./history/isa.md) | The first ISA design; `bytecode.md` folded it in. |
| [`bytecode-prior-art.md`](./history/bytecode-prior-art.md) | Zend, Lua and other prior art behind the ISA. |
| [`ast-surface.md`](./history/ast-surface.md) | The mago-syntax CST catalogue the tree-walker matched on. |
| [`builtins-min.md`](./history/builtins-min.md) | The minimal builtin set of the first language phases. |
| [`oop-ast.md`](./history/oop-ast.md) | The OOP CST catalogue; folded into `oop.md`. |
| [`oop-isa.md`](./history/oop-isa.md) | The OOP opcode design; folded into `oop.md`. |
| [`oop-prior-art.md`](./history/oop-prior-art.md) | Zend and V8 object models and dispatch, input to `oop.md`. |
| [`modules-and-streams.md`](./history/modules-and-streams.md) | The module, stream-wrapper and phar plan for running `composer.phar` (implemented). |
