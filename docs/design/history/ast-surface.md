# mago-syntax AST Surface Catalog (rhp interpreter contract)

Exhaustive enumeration of the mago-syntax **1.58.2** CST that the rhp tree-walking
interpreter matches against for language phases 1–7 (output/echo, scalars &
expressions, control flow, arrays, functions, references, OOP).

- Source of truth: vendored at
  `/Users/robing/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/mago-syntax-1.58.2/src`
  (read directly; do not trust docs.rs).
- Target semantics: PHP 8.5 (local `php` is 8.5.8; use it as ground truth).
- All types are re-exported flat from `mago_syntax::cst::*` (see `src/cst/cst/mod.rs`),
  so `use mago_syntax::cst::{Statement, Expression, ...}` reaches every node.

## 0. Arena, ownership, and entry point

```rust
// src/parser/mod.rs
pub fn parse_file_content<'arena, A: Arena>(
    arena: &'arena A, file_id: FileId, content: &[u8],
) -> &'arena Program<'arena>;
```

```rust
// src/cst/mod.rs
pub struct Program<'arena> {
    pub file_id: FileId,
    pub source_text: &'arena [u8],
    pub trivia: Sequence<'arena, Trivia<'arena>>,     // comments/whitespace — ignore for exec
    pub statements: Sequence<'arena, Statement<'arena>>, // <-- interpreter entry
    pub errors: &'arena [ParseError],                 // check has_errors() before running
}
```

**Ownership contract.** Everything is borrowed from the arena: `Statement<'arena>`,
`Expression<'arena>`, `&'arena [u8]`, `&'arena Expression`, `Sequence`,
`TokenSeparatedSequence` all live as long as `'arena`. **Runtime `Value`s must be
OWNED** (`String`/`Vec<u8>`, owned numbers, `Rc`/`RefCell` for refs/objects): copy
bytes out of `&'arena [u8]` into owned storage the moment a value is produced, because
a value stored in a variable/array must outlive the statement that created it. Spans
(`Span`, `FileId`, `Position`) are `Copy` and carry no payload the interpreter needs
besides error reporting.

Notable: `Expression<'arena>` and many node types hold `&'arena Expression<'arena>`
for children, so recursion is cheap (no boxing) but you must deref the reference.

## 1. Sequence / TokenSeparatedSequence iteration API

Defined in `mago_syntax_core::cst` (`mago-syntax-core-1.58.2/src/cst/mod.rs`),
aliased in `src/cst/sequence.rs`.

```rust
pub struct Sequence<'arena, T> { pub nodes: &'arena [T] }                 // Copy
pub struct TokenSeparatedSequence<'arena, T, Tok> {                       // PHP: Tok = Token
    pub nodes: &'arena [T], pub tokens: &'arena [Tok],
}
```

Both expose: `len()`, `is_empty()`, `get(i)`, `first()`, `last()`, `iter()` (→
`std::slice::Iter<'_, T>`), `as_slice()`, `Index<usize>`, and `IntoIterator` (by value
yields `&'arena T`, by ref yields `&T`). **For the interpreter just use `.iter()` /
`.as_slice()` and ignore the separator `tokens`** — they are only needed for
formatting/trailing-comma detection (`has_trailing_token`, `get_trailing_token`,
`iter_with_tokens`). `TokenSeparatedSequence<'arena, T>` is the PHP alias (separator
fixed to `Token<'arena>`); span helpers live in trait `TokenSeparatedSequenceExt`.

Gotcha: a `TokenSeparatedSequence` can hold a trailing separator with no node (e.g.
`[1, 2,]`), and can even be empty-with-tokens — always drive loops off `nodes`.

---

## 2. `enum Statement<'arena>` — `src/cst/cst/statement.rs`

`#[non_exhaustive]` — match arms need a `_ =>` fallback. 32 variants.

| Variant | Payload | Meaning / interpreter gotcha |
|---|---|---|
| `OpeningTag(OpeningTag<'arena>)` | `Full(FullOpeningTag{span,value})` or `Short(ShortOpeningTag{span})` | `<?php` / `<?`. Pure lexical marker — **no-op at exec**; does not produce output. |
| `ClosingTag(ClosingTag)` | `{span}` | `?>`. No-op for exec; see terminator/HTML note §12. |
| `Inline(Inline<'arena>)` | `{kind: InlineKind, span, value: &[u8]}` | Raw text/HTML outside PHP tags. **Echo `value` verbatim to stdout.** `InlineKind::{Text, Shebang}` — a `Shebang` (`#!...` on line 1) must NOT be printed. |
| `Namespace(Namespace<'arena>)` | see §11 | `namespace Foo;` or `namespace Foo { ... }`. |
| `Use(Use<'arena>)` | see §11 | `use` import. Phase 7: affects name resolution only, no runtime effect. |
| `Class(Class<'arena>)` | see §10 | Class declaration. |
| `Interface(Interface<'arena>)` | see §10 | Interface declaration. |
| `Trait(Trait<'arena>)` | see §10 | Trait declaration. |
| `Enum(Enum<'arena>)` | see §10 | Enum declaration. |
| `Block(Block<'arena>)` | `{left_brace, statements: Sequence<Statement>, right_brace}` | A bare `{ ... }` block. **Does NOT introduce a scope in PHP** — just run the statements. |
| `Constant(Constant<'arena>)` | see §11 | `const FOO = 1, BAR = 2;` (file/namespace-level). |
| `Function(Function<'arena>)` | see §9 | Named function declaration; register it, hoisted. |
| `Declare(Declare<'arena>)` | see §11 | `declare(strict_types=1);` etc. Phase 1: honor `strict_types`, else no-op. |
| `Goto(Goto<'arena>)` | `{goto, label: LocalIdentifier, terminator}` | `goto label;`. Low priority / may defer. |
| `Label(Label<'arena>)` | `{name: LocalIdentifier, colon}` | `label:` goto target. |
| `Try(Try<'arena>)` | see §8 | try/catch/finally. |
| `Foreach(Foreach<'arena>)` | see §7 | foreach loop. |
| `For(For<'arena>)` | see §7 | for loop. |
| `While(While<'arena>)` | see §7 | while loop. |
| `DoWhile(DoWhile<'arena>)` | see §7 | do/while loop. |
| `Continue(Continue<'arena>)` | `{continue, level: Option<&Expression>, terminator}` | `continue;` / `continue 2;`. `level` is an expression (usually int literal). |
| `Break(Break<'arena>)` | `{break, level: Option<&Expression>, terminator}` | `break;` / `break 2;`. |
| `Switch(Switch<'arena>)` | see §8 | switch statement. |
| `If(If<'arena>)` | see §8 | if/elseif/else (both brace and `:`/`endif` forms). |
| `Return(Return<'arena>)` | `{return, value: Option<&Expression>, terminator}` | `return;` / `return $x;`. `value` None = return null (or void). |
| `Expression(ExpressionStatement<'arena>)` | `{expression: &Expression, terminator}` | Any expression used as a statement (`$a = 1;`, `foo();`). **Most common statement.** |
| `Echo(Echo<'arena>)` | `{echo: Keyword, values: TokenSeparatedSequence<&Expression>, terminator}` | `echo $a, $b;` — **iterate `values`, stringify each, output in order.** |
| `EchoTag(EchoTag<'arena>)` | `{tag: Span, values: TokenSeparatedSequence<&Expression>, terminator}` | `<?= $a, $b ?>` short-echo tag. Same behavior as `Echo`. |
| `Global(Global<'arena>)` | `{global, variables: TokenSeparatedSequence<Variable>, terminator}` | `global $x, $y;` — bind names to the global scope (phase 6, references). |
| `Static(Static<'arena>)` | see §6 | `static $x = 1;` function-static variables. |
| `HaltCompiler(HaltCompiler<'arena>)` | `{halt_compiler, left_parenthesis, right_parenthesis, terminator}` | `__halt_compiler();` — stop executing/parsing the rest of the file. |
| `Unset(Unset<'arena>)` | `{unset, left_parenthesis, values: TokenSeparatedSequence<&Expression>, right_parenthesis, terminator}` | `unset($a, $b[0]);` — remove variables/array elements/props. |
| `Noop(Span)` | `Span` | A lone `;`. Do nothing. |

Helper methods on `Statement`: `is_closing_tag`, `terminates_scripting`, `is_loop`,
`is_control_flow`, `is_declaration`, `is_noop`. `HasSpan` is implemented for every
variant.

Declaration **hoisting**: PHP hoists top-level `function`/`class`/etc. declarations
within a scope. Do a pre-pass over a statement list to register `Function`, `Class`,
`Interface`, `Trait`, `Enum` before executing, to match PHP’s forward-reference rules
(conditional declarations are NOT hoisted — only unconditional top-of-scope ones).

---

## 3. `enum Expression<'arena>` — `src/cst/cst/expression.rs`

`#[non_exhaustive]` — needs `_ =>` fallback. 33 variants. Each child pointer is
`&'arena Expression<'arena>` unless noted.

| Variant | Payload | Meaning / gotcha |
|---|---|---|
| `Binary(Binary<'arena>)` | `{lhs, operator: BinaryOperator, rhs}` | Binary op. See §4 for the full operator set. `&&`/`\|\|`/`and`/`or` short-circuit; `??` short-circuits & suppresses undefined-var notices. |
| `UnaryPrefix(UnaryPrefix<'arena>)` | `{operator: UnaryPrefixOperator, operand}` | Prefix op OR **cast** OR `@` OR `&ref` OR `++`/`--`. See §5. |
| `UnaryPostfix(UnaryPostfix<'arena>)` | `{operand, operator: UnaryPostfixOperator}` | `$x++` / `$x--`. |
| `Parenthesized(Parenthesized<'arena>)` | `{left_parenthesis, expression: &Expression, right_parenthesis}` | `( expr )`. **Transparent — just eval inner.** Helpers like `unparenthesized()`, `is_assignment()` already see through it. |
| `Literal(Literal<'arena>)` | see §4.0 | scalar literal. |
| `CompositeString(CompositeString<'arena>)` | see §4.1 | interpolated `"..."`, heredoc/nowdoc, or backtick shell. |
| `Assignment(Assignment<'arena>)` | `{lhs, operator: AssignmentOperator, rhs}` | `=`, `+=`, `.=`, `??=`, ... and `=&` (ref assign, where rhs is `UnaryPrefix{Reference}`). See §5. Assignment is an expression and yields the assigned value. |
| `Conditional(Conditional<'arena>)` | `{condition, question_mark, then: Option<&Expression>, colon, else: &Expression}` | `c ? t : e`. **`then == None` means the elvis `c ?: e`** (reuse condition's value). |
| `Array(Array<'arena>)` | `{left_bracket, elements: TokenSeparatedSequence<ArrayElement>, right_bracket}` | `[...]` literal. See §7.0. |
| `LegacyArray(LegacyArray<'arena>)` | `{array: Keyword, left_parenthesis, elements, right_parenthesis}` | `array(...)` literal. Same element enum as `Array`. |
| `List(List<'arena>)` | `{list: Keyword, left_parenthesis, elements, right_parenthesis}` | `list(...)` destructuring target (LHS of `=`). Same `ArrayElement` enum; `Missing` elements are skipped slots. Note `[...]` on the LHS also destructures (an `Array` used as assignment target). |
| `ArrayAccess(ArrayAccess<'arena>)` | `{array, left_bracket, index, right_bracket}` | `$a[$k]`. As LHS: may auto-vivify. |
| `ArrayAppend(ArrayAppend<'arena>)` | `{array, left_bracket, right_bracket}` | `$a[]` — append target. **Only valid as an assignment LHS / by-ref target;** no index. |
| `AnonymousClass(AnonymousClass<'arena>)` | see §10 | `new class(...) extends ... {...}`. |
| `Closure(Closure<'arena>)` | see §9 | `function (...) use (...) {...}`. |
| `ArrowFunction(ArrowFunction<'arena>)` | see §9 | `fn(...) => expr` — auto-captures by value. |
| `Variable(Variable<'arena>)` | see §6 | `$x`, `${expr}`, `$$x`. |
| `ConstantAccess(ConstantAccess<'arena>)` | `{name: Identifier}` | Bareword global constant reference (`PHP_EOL`, `FOO`). Distinct from `Identifier`. |
| `Identifier(Identifier<'arena>)` | see §4.2 | A bare name used as an expression (usually a callee in `FunctionCall.function`, or a class name). Local/Qualified/FullyQualified. |
| `Match(Match<'arena>)` | see §8 | `match (x) { ... }` expression (strict `===` comparison, throws `\UnhandledMatchError` if no arm). |
| `Yield(Yield<'arena>)` | see §9 | generator yield / `yield from`. |
| `Construct(Construct<'arena>)` | see §4.3 | language constructs: isset/empty/print/exit/die/include*/require*/**eval**. |
| `Throw(Throw<'arena>)` | `{throw, exception: &Expression}` | `throw $e` — in 8.x this is an **expression** (usable in `?:`, arrow fns). |
| `Clone(Clone<'arena>)` | `{clone, object: &Expression}` | `clone $obj`. |
| `Call(Call<'arena>)` | see §9 | function/method/static/nullsafe-method call. |
| `PartialApplication(PartialApplication<'arena>)` | see §9 | First-class callable `foo(...)` and PFA `foo(1, ?)`. |
| `Access(Access<'arena>)` | see §10 | property / nullsafe-property / static-property / class-constant access. |
| `Parent(Keyword<'arena>)` | `Keyword` | `parent` used as expression (in `parent::...`). |
| `Static(Keyword<'arena>)` | `Keyword` | `static` used as expression (late static binding, `static::...`, `new static`). |
| `Self_(Keyword<'arena>)` | `Keyword` | `self` used as expression (`self::...`, `new self`). |
| `Instantiation(Instantiation<'arena>)` | `{new: Keyword, class: &Expression, argument_list: Option<ArgumentList>}` | `new Foo(...)`. `class` may be an `Identifier`, `Variable`, `Self_/Static/Parent`, etc. `argument_list == None` for `new Foo` (no parens). |
| `MagicConstant(MagicConstant<'arena>)` | see §4.4 | `__LINE__`, `__FILE__`, `__CLASS__`, ... |
| `Pipe(Pipe<'arena>)` | `{input, operator: Span, callable}` | `$x \|> $fn` (PHP 8.5 pipe operator). Evaluate `input`, call `callable` with it as sole arg. |
| `Error(Span)` | `Span` | Parser-produced error placeholder. Should not appear when `Program::has_errors()` is false; treat as a hard error. |

Helpers worth reusing: `unparenthesized()`, `is_assignment/is_call/is_variable/
is_binary/is_unary/is_conditional/is_reference/is_true/is_false/is_literal/
is_string_literal/is_throw`, `evaluates_to_boolean()`, `is_referenceable(include_calls)`,
`get_array_like_elements()`, `is_constant(version, init)`, `node_kind()`. `HasSpan`
implemented for all.

---

## 4. Literals, strings, identifiers, constructs, magic constants

### 4.0 `enum Literal<'arena>` — `src/cst/cst/literal.rs`

| Variant | Payload | Gotcha |
|---|---|---|
| `String(LiteralString<'arena>)` | `{kind: LiteralStringKind, span, raw: &[u8], value: Option<&[u8]>}` | `'...'`/`"..."` **with no interpolation**. `raw` includes quotes & escapes; **`value` is the already-unescaped byte content** — use `value` when present. `value: None` means unescaping failed/needs deferral (fall back to decoding `raw` yourself). `kind ∈ {SingleQuoted, DoubleQuoted}`. |
| `Integer(LiteralInteger<'arena>)` | `{span, raw: &[u8], value: Option<u64>}` | `value: Some(n)` is the parsed magnitude as **u64**. `value: None` ⇒ the literal overflows `i64`/`u64` and **PHP promotes it to float** — handle that. Decimal/hex/octal/binary all land here; `raw` keeps the original text (incl. `_` separators, `0x`/`0b`/`0o`). |
| `Float(LiteralFloat<'arena>)` | `{span, raw, value: OrderedFloat<f64>}` | `value` always present. |
| `True(Keyword<'arena>)` | keyword | `true` (case-insensitive). |
| `False(Keyword<'arena>)` | keyword | `false`. |
| `Null(Keyword<'arena>)` | keyword | `null`. |

### 4.1 `enum CompositeString<'arena>` — `src/cst/cst/string.rs`

Any string with runtime parts. `parts()` returns `&Sequence<StringPart>`.

| Variant | Payload |
|---|---|
| `ShellExecute(ShellExecuteString)` | `` `...` `` backticks. **HIGH RISK — do NOT execute; refuse/stub** (shell exec). `{left_backtick, parts, right_backtick}` |
| `Interpolated(InterpolatedString)` | `{prefix: Option<Keyword>, left_double_quote, parts, right_double_quote}` double-quoted with `$var`/`{...}`. |
| `Document(DocumentString)` | `{prefix, open, kind: DocumentKind, indentation: DocumentIndentation, label: &[u8], parts, close}` heredoc/nowdoc. `kind ∈ {Heredoc, Nowdoc}` (Nowdoc = no interpolation). `indentation ∈ {None, Whitespace(n), Tab(n), Mixed(ws,tab)}` — **strip that leading indentation from each line** (PHP 7.3+ flexible heredoc). |

`enum StringPart<'arena>`:
- `Literal(LiteralStringPart{span, raw, value: Option<&[u8]>})` — literal chunk; same
  `raw`/`value` convention as `LiteralString` (use `value` unescaped when `Some`).
- `Expression(&'arena Expression)` — simple interpolation `$x`, `$x->y`, `$x[k]`.
- `BracedExpression(BracedExpressionStringPart{left_brace, expression, right_brace})` —
  `{$complex->expr()}` interpolation.
Concatenate the stringified parts in order.

### 4.2 `enum Identifier<'arena>` — `src/cst/cst/identifier.rs`

| Variant | Payload | |
|---|---|---|
| `Local(LocalIdentifier{span, value: &[u8]})` | `foo` | unqualified |
| `Qualified(QualifiedIdentifier{span, value})` | `Foo\bar` | relative namespace path |
| `FullyQualified(FullyQualifiedIdentifier{span, value})` | `\Foo\bar` | leading `\` |

`value()` returns the raw bytes; `last_segment()` returns the final `\`-segment.
PHP identifiers (function/const names) are **case-insensitive**; class names too.

### 4.3 `enum Construct<'arena>` — `src/cst/cst/construct.rs`

Special language constructs (parsed distinctly from function calls).

| Variant | Payload | Gotcha / risk |
|---|---|---|
| `Isset(IssetConstruct)` | `{isset, left_parenthesis, values: TokenSeparatedSequence<&Expression>, right_parenthesis}` | `isset($a, $b)` — must NOT trigger undefined notices; true iff all set and non-null. |
| `Empty(EmptyConstruct)` | `{empty, (, value, )}` | `empty($x)` — falsy check without notices. |
| `Eval(EvalConstruct)` | `{eval, (, value, )}` | **HIGH RISK — NOT SUPPORTED. Refuse at runtime.** (arbitrary code exec) |
| `Include(IncludeConstruct)` | `{include, value}` | file include — phase deferred; filesystem/code-load risk. |
| `IncludeOnce(IncludeOnceConstruct)` | `{include_once, value}` | as above. |
| `Require(RequireConstruct)` | `{require, value}` | as above (fatal on failure). |
| `RequireOnce(RequireOnceConstruct)` | `{require_once, value}` | as above. |
| `Print(PrintConstruct)` | `{print, value}` | `print $x` — outputs, evaluates to int `1`. |
| `Exit(ExitConstruct)` | `{exit, arguments: Option<ArgumentList>}` | `exit`/`exit($code)`/`exit("msg")`. Terminates; string arg printed, int arg = status. |
| `Die(DieConstruct)` | `{die, arguments: Option<ArgumentList>}` | alias of `exit`. |

`is_import()` groups include/require; `has_bounds()` false for the import+print ones
(they have no parentheses/closing bound).

### 4.4 `enum MagicConstant<'arena>` — `src/cst/cst/magic_constant.rs`

Each holds a `LocalIdentifier`. Resolve at compile/eval time from context:
`Line`(`__LINE__`), `File`(`__FILE__`), `Directory`(`__DIR__`), `Trait`(`__TRAIT__`),
`Method`(`__METHOD__`), `Function`(`__FUNCTION__`), `Property`(`__PROPERTY__`),
`Namespace`(`__NAMESPACE__`), `Class`(`__CLASS__`).

---

## 5. Operators (exact enum paths)

### 5.1 `enum BinaryOperator<'arena>` — `mago_syntax::cst::BinaryOperator` (`binary.rs`)

`Copy`. Most variants carry a `Span`; `instanceof`/`and`/`or`/`xor` carry a `Keyword`.

Arithmetic: `Addition` `+`, `Subtraction` `-`, `Multiplication` `*`, `Division` `/`,
`Modulo` `%`, `Exponentiation` `**`.
Bitwise: `BitwiseAnd` `&`, `BitwiseOr` `|`, `BitwiseXor` `^`, `LeftShift` `<<`,
`RightShift` `>>`.
Null coalesce: `NullCoalesce` `??` (short-circuits, suppresses undefined notices).
Equality/comparison: `Equal` `==`, `NotEqual` `!=`, `Identical` `===`,
`NotIdentical` `!==`, `AngledNotEqual` `<>` (same as `!=`), `LessThan` `<`,
`LessThanOrEqual` `<=`, `GreaterThan` `>`, `GreaterThanOrEqual` `>=`,
`Spaceship` `<=>`.
String: `StringConcat` `.`.
Type: `Instanceof(Keyword)` `instanceof`.
Logical: `And` `&&`, `Or` `||`, `LowAnd(Keyword)` `and`, `LowOr(Keyword)` `or`,
`LowXor(Keyword)` `xor` (the `Low*` forms have very low precedence — matters only for
precedence, already resolved into the tree; evaluate `xor` as boolean XOR).

Helpers: `is_arithmetic/is_additive/is_multiplicative/is_bit_shift/is_bitwise/
is_equality/is_identity/is_comparison/is_logical/is_low_precedence/is_concatenation/
is_null_coalesce/is_instanceof/is_negated_equality`, `as_bytes()`, `precedence()`.
**Precedence/associativity are already baked into the tree** — do not re-apply; just
evaluate. Short-circuit `&&`/`||`/`and`/`or`/`??` by not evaluating `rhs` when
decided.

### 5.2 `enum UnaryPrefixOperator<'arena>` — `unary.rs`

Non-cast: `ErrorControl(Span)` `@` (suppress diagnostics on operand),
`Reference(Span)` `&` (reference-of — appears in `=&`, by-ref foreach/args/array elems),
`BitwiseNot(Span)` `~`, `Not(Span)` `!`, `PreIncrement(Span)` `++`,
`PreDecrement(Span)` `--`, `Plus(Span)` `+` (unary plus), `Negation(Span)` `-`.

**Casts** (each is `(Span, &'arena [u8])`, the `&[u8]` being the raw cast text):
`ArrayCast` `(array)`, `BoolCast` `(bool)`, `BooleanCast` `(boolean)`,
`DoubleCast` `(double)`, `RealCast` `(real)`, `FloatCast` `(float)`, `IntCast` `(int)`,
`IntegerCast` `(integer)`, `ObjectCast` `(object)`, `UnsetCast` `(unset)`,
`StringCast` `(string)`, `BinaryCast` `(binary)`, `VoidCast` `(void)`.

> **PHP 8.5 cast reality (verified against `php` 8.5.8) — the parser is permissive,
> the engine is not:**
> - Canonical & supported: `(int)`, `(bool)`, `(float)`, `(string)`, `(array)`, `(object)`.
> - Deprecated but still functional (emit `E_DEPRECATED`, behave as canonical):
>   `(integer)`→int, `(boolean)`→bool, `(double)`→float, `(binary)`→string.
> - `(unset)` → **fatal `Error` at runtime** ("no longer supported"). Mago parses it;
>   rhp should raise a fatal, not perform it.
> - `(real)` and `(void)` → **PHP parse errors** in 8.5 (`(real)` removed; `(void)` not
>   a cast). Mago has `RealCast`/`VoidCast` variants for older-version parsing; under
>   our 8.5 target treat them as errors if ever produced.

Helpers: `is_error_control/is_cast/is_reference/is_arithmetic/
is_increment_or_decrement/is_not`, `as_bytes()`, `precedence()`.

### 5.3 `enum UnaryPostfixOperator` — `unary.rs` (no lifetime)

`PostIncrement(Span)` `$x++`, `PostDecrement(Span)` `$x--`. `as_str()` available.
Semantics: yield old value, then mutate the operand lvalue.

### 5.4 `enum AssignmentOperator` — `assignment.rs` (no lifetime)

`Assign` `=`, `Addition` `+=`, `Subtraction` `-=`, `Multiplication` `*=`,
`Division` `/=`, `Modulo` `%=`, `Exponentiation` `**=`, `Concat` `.=`,
`BitwiseAnd` `&=`, `BitwiseOr` `|=`, `BitwiseXor` `^=`, `LeftShift` `<<=`,
`RightShift` `>>=`, `Coalesce` `??=` (assign only if LHS null/unset; short-circuits RHS).
Each carries a `Span`. `as_str()`, `is_assign()`, `is_arithmetic()`, `is_bitwise()`.
**Reference assignment `$a =& $b`** is `Assign` with `rhs` an
`Expression::UnaryPrefix{ operator: Reference, operand }` — detect via
`rhs.is_reference()`.

---

## 6. Variables, references, static — `variable.rs`, `static.rs`

`enum Variable<'arena>`:
- `Direct(DirectVariable{span, name: &[u8]})` — `$foo`. **`name` INCLUDES the leading
  `$`** (verified: the lexer consumes `1 + ident_len` from `$`, so `$foo` ⇒
  `name == b"$foo"`). Strip the `$` when keying your variable table. This is the
  common case.
- `Indirect(IndirectVariable{dollar_left_brace, expression, right_brace})` — `${expr}`:
  evaluate `expression` to a string, use as var name.
- `Nested(NestedVariable{dollar, variable: &Variable})` — `$$foo` / `$$$foo`: evaluate
  inner variable to get the name, then look that up (variable variables).

`$this` is a `DirectVariable` with `name == b"$this"` — bound inside methods.

`enum StaticItem<'arena>` (items of a `static $x = ...;` statement):
- `Abstract(StaticAbstractItem{variable: DirectVariable})` — `static $x;`
- `Concrete(StaticConcreteItem{variable, equals, value})` — `static $x = expr;`
  (initializer runs once per function; persists across calls). `variable()`/`value()`.

References (phase 6): the `&` operator is `UnaryPrefixOperator::Reference`; by-ref
appears in: `=&` assignment, foreach `&$v`, params (`ampersand: Option<Span>`), closure
`use (&$v)` (`ClosureUseClauseVariable.ampersand`), array elements, and
`Global`/`Static`. Runtime model: store variables as shared `Rc<RefCell<Value>>` slots
so aliases share storage.

---

## 7. Arrays & loops

### 7.0 `enum ArrayElement<'arena>` — `array.rs` (shared by Array/LegacyArray/List)

- `KeyValue(KeyValueArrayElement{key, double_arrow, value})` — `k => v`.
- `Value(ValueArrayElement{value})` — bare `v`.
- `Variadic(VariadicArrayElement{ellipsis, value})` — `...$spread` (spread/unpack).
- `Missing(MissingArrayElement{comma})` — an empty slot `[a, , c]` (only meaningful in
  `list()`/destructuring, where it skips a position). `get_key()`/`get_value()` return
  `None` for Missing. Helpers: `is_variadic/is_missing/is_key_value/is_value`.

`ArrayAccess{array, index}`, `ArrayAppend{array}` (no index — append target).
Arrays are **ordered maps** (int+string keys); keys normalize per PHP rules (numeric
strings → int, bool/null/float → int/empty-string). As LHS, `$a[$k]`/`$a[]` auto-vivify.

### 7.1 Foreach — `src/cst/cst/loop/foreach.rs`

```
Foreach { foreach, (, expression, as: Keyword, target: ForeachTarget, ), body: ForeachBody }
```
`enum ForeachTarget`:
- `Value(ForeachValueTarget{value})` — `as $v`.
- `KeyValue(ForeachKeyValueTarget{key, double_arrow, value})` — `as $k => $v`.
`target.value()` may itself be `&$v` (by-ref, a `UnaryPrefix{Reference}`) or a `List`/
`Array` (destructuring). `enum ForeachBody`:
- `Statement(&Statement)` — single statement body.
- `ColonDelimited(ForeachColonDelimitedBody{colon, statements, end_foreach, terminator})`
  — `foreach(...): ... endforeach;`.
`body.statements()` normalizes both to a `&[Statement]`.

### 7.2 For — `loop/for.rs`

```
For { for, (, initializations: TokenSeparatedSequence<&Expression>, initializations_semicolon,
      conditions: TokenSeparatedSequence<&Expression>, conditions_semicolon,
      increments: TokenSeparatedSequence<&Expression>, ), body: ForBody }
```
Each of the three clauses is a **comma-separated list of expressions** (all inits run;
**the loop condition is the LAST expression in `conditions`**, others are evaluated for
side effects; all increments run). Empty `conditions` ⇒ infinite loop.
`enum ForBody { Statement(&Statement), ColonDelimited(ForColonDelimitedBody{..,end_for,..}) }`;
`body.statements()`.

### 7.3 While / DoWhile — `loop/while.rs`, `loop/do_while.rs`

```
While { while, (, condition, ), body: WhileBody }           // body.statements()
WhileBody { Statement(&Statement), ColonDelimited(WhileColonDelimitedBody{.., end_while, ..}) }
DoWhile { do, statement: &Statement, while, (, condition, ), terminator }  // body runs >=1x
```

### 7.4 Break/Continue — `loop/mod.rs`

`Break{break, level: Option<&Expression>, terminator}`,
`Continue{continue, level: Option<&Expression>, terminator}`. `level` (e.g. `break 2;`)
selects how many enclosing loop/switch levels to exit/skip; default 1. Implement via a
control-flow signal propagated up (e.g. `Flow::Break(n)` / `Flow::Continue(n)`), and
note **`switch` counts as a level** for break/continue.

---

## 8. If / Switch / Match / Try

### 8.1 If — `control_flow/if.rs`

```
If { if, (, condition, ), body: IfBody }
IfBody::Statement(IfStatementBody{
    statement: &Statement,
    else_if_clauses: Sequence<IfStatementBodyElseIfClause{elseif,(,condition,),statement}>,
    else_clause: Option<IfStatementBodyElseClause{else, statement}>,
})
IfBody::ColonDelimited(IfColonDelimitedBody{   // if(): ... elseif(): ... else: ... endif;
    colon, statements: Sequence<Statement>,
    else_if_clauses: Sequence<IfColonDelimitedBodyElseIfClause{elseif,(,cond,),colon,statements}>,
    else_clause: Option<IfColonDelimitedBodyElseClause{else, colon, statements}>,
    endif: Keyword, terminator,
})
```
Convenience on `IfBody`: `statements()`, `else_statements()`, `else_if_statements()`,
`else_if_clauses() -> Vec<(&Expression, &[Statement])>` — **use `else_if_clauses()` to
iterate uniformly across both body forms.**

### 8.2 Switch — `control_flow/switch.rs`

```
Switch { switch, (, expression, ), body: SwitchBody }
SwitchBody::BraceDelimited(SwitchBraceDelimitedBody{left_brace, optional_terminator, cases, right_brace})
SwitchBody::ColonDelimited(SwitchColonDelimitedBody{colon, optional_terminator, cases, end_switch, terminator})
```
`body.cases() -> &[SwitchCase]`, `has_default_case()`.
`enum SwitchCase`:
- `Expression(SwitchExpressionCase{case, expression, separator: SwitchCaseSeparator, statements})`
- `Default(SwitchDefaultCase{default, separator, statements})`
`SwitchCaseSeparator ∈ {Colon(Span), SemiColon(Span)}` (`case 1:` or `case 1;`).
Semantics: **loose `==` comparison**, **fall-through** between cases unless `break`.
`case.statements()`, `is_default()`, `is_fall_through()` (helper: true if non-empty and
last stmt isn't `break`). Evaluate matching case, then execute all subsequent cases'
statements until a break/return.

### 8.3 Match — `control_flow/match.rs`

```
Match { match, (, expression, ), left_brace,
        arms: TokenSeparatedSequence<MatchArm>, right_brace }
MatchArm::Expression(MatchExpressionArm{conditions: TokenSeparatedSequence<&Expression>, arrow, expression})
MatchArm::Default(MatchDefaultArm{default, comma: Option<Span>, arrow, expression})
```
It is an **expression**. Strict **`===`** comparison against each condition (a single
arm may list several `conditions`). First match's `expression` is the result. No
fall-through. **No match and no default ⇒ throw `\UnhandledMatchError`.** `arm.expression()`,
`is_default()`, `is_conditional()`.

### 8.4 Try — `try.rs`

```
Try { try, block: Block, catch_clauses: Sequence<TryCatchClause>, finally_clause: Option<TryFinallyClause> }
TryCatchClause { catch, (, hint: Hint, variable: Option<DirectVariable>, ), block: Block }
TryFinallyClause { finally, block: Block }
```
`catch` `hint` is a type (`Hint`, often a `Union` like `A|B`); `variable` is optional
(`catch (Exception)` with no `$e` is legal). `finally` always runs (even on
return/throw within try/catch). Match thrown object against each catch's type union in
order.

---

## 9. Functions, closures, calls, params, yield

### 9.1 `Function` — `function_like/function.rs`

```
Function { attribute_lists: Sequence<AttributeList>, function: Keyword,
           ampersand: Option<Span>,          // returns by reference
           name: LocalIdentifier,
           parameter_list: FunctionLikeParameterList,
           return_type_hint: Option<FunctionLikeReturnTypeHint>,
           body: Block }
```

### 9.2 `Closure` — `function_like/closure.rs`

```
Closure { attribute_lists, r#static: Option<Keyword>,   // static fn: no $this binding
          function: Keyword, ampersand: Option<Span>,
          parameter_list, use_clause: Option<ClosureUseClause>,
          return_type_hint, body: Block }
ClosureUseClause { use, (, variables: TokenSeparatedSequence<ClosureUseClauseVariable>, ) }
ClosureUseClauseVariable { ampersand: Option<Span>, variable: DirectVariable }  // &$v = by-ref capture
```
`use` captures are **by value at closure-creation time** unless `ampersand` present.

### 9.3 `ArrowFunction` — `function_like/arrow_function.rs`

```
ArrowFunction { attribute_lists, r#static: Option<Keyword>, fn: Keyword,
                ampersand: Option<Span>, parameter_list,
                return_type_hint, arrow: Span, expression: &Expression }
```
**Auto-captures by value** any outer variable used in `expression` (no `use`). Body is a
single expression, implicitly returned.

### 9.4 Parameters — `function_like/parameter.rs`

```
FunctionLikeParameterList { (, parameters: TokenSeparatedSequence<FunctionLikeParameter>, ) }
FunctionLikeParameter {
    attribute_lists, modifiers: Sequence<Modifier>,     // non-empty => CONSTRUCTOR PROMOTION
    hint: Option<Hint>,
    ampersand: Option<Span>,                            // by-reference param
    ellipsis: Option<Span>,                             // variadic ...$args
    variable: DirectVariable,
    default_value: Option<FunctionLikeParameterDefaultValue{equals, value}>,
    hooks: Option<PropertyHookList>,                    // promoted property hooks
}
```
`is_promoted_property()` (has modifiers or hooks), `is_variadic()`, `is_reference()`.
Binding: positional then named args (see §9.6); defaults evaluated lazily when arg
missing; variadic collects the rest into an array.

### 9.5 `enum Call<'arena>` — `call.rs`

| Variant | Payload |
|---|---|
| `Function(FunctionCall{function: &Expression, argument_list})` | `foo(...)` / `$cb(...)` / `("Foo\\bar")(...)`. `function` is any expression (usually `Identifier`/`Variable`). |
| `Method(MethodCall{object, arrow, method: ClassLikeMemberSelector, argument_list})` | `$o->m(...)`. |
| `NullSafeMethod(NullSafeMethodCall{object, question_mark_arrow, method, argument_list})` | `$o?->m(...)` — short-circuits to null if `object` is null. |
| `StaticMethod(StaticMethodCall{class, double_colon, method, argument_list})` | `C::m(...)` / `self::`/`static::`/`parent::`. |

`get_argument_list()`, `is_null_safe()`. `method` is a `ClassLikeMemberSelector` (§10).

### 9.6 Arguments — `argument.rs`

```
ArgumentList { (, arguments: TokenSeparatedSequence<Argument>, ) }
Argument::Positional(PositionalArgument{ellipsis: Option<Span>, value})   // ...$spread if ellipsis
Argument::Named(NamedArgument{name: LocalIdentifier, colon, value})       // name: value
```
`is_positional()`, `is_unpacked()` (positional with ellipsis = spread), `value()`.
Named args bind by parameter name; spread unpacks array/Traversable.

### 9.7 `enum PartialApplication<'arena>` — `partial_application.rs`

First-class callable syntax `foo(...)` and PFA `foo(1, ?, ...)` (uses
`PartialArgumentList`). Variants `Function`/`Method`/`StaticMethod` mirroring `Call`.
`is_first_class_callable()` (single `VariadicPlaceholder` arg ⇒ `foo(...)`, produces a
`Closure`). PFA `PartialArgument` adds `NamedPlaceholder`, `Placeholder` (`?`),
`VariadicPlaceholder` (`...`). **FCC is in scope for phase 5; full PFA with placeholders
can be deferred.**

### 9.8 `enum Yield<'arena>` — `yield.rs` (generators, phase 5+/deferred)

- `Value(YieldValue{yield, value: Option<&Expression>})` — `yield` / `yield $v`.
- `Pair(YieldPair{yield, key, arrow, value})` — `yield $k => $v`.
- `From(YieldFrom{yield, from, iterator})` — `yield from $iter`.
Presence of any `yield` in a function body makes it a generator (lazy). May defer.

`FunctionLikeReturnTypeHint { colon, hint: Hint }` (§ shared, `function_like/return.rs`).

---

## 10. OOP: classes, members, access, selectors — `class_like/`

### 10.1 Declarations — `class_like/mod.rs`

```
Class { attribute_lists, modifiers: Sequence<Modifier>, class: Keyword, name: LocalIdentifier,
        extends: Option<Extends>, implements: Option<Implements>,
        left_brace, members: Sequence<ClassLikeMember>, right_brace }
Interface { attribute_lists, interface, name, extends: Option<Extends>, {, members, } }
Trait { attribute_lists, trait, name, {, members, } }
Enum { attribute_lists, enum, name, backing_type_hint: Option<EnumBackingTypeHint{colon,hint}>,
       implements: Option<Implements>, {, members, } }
AnonymousClass { new, attribute_lists, modifiers, class, argument_list: Option<PartialArgumentList>,
                 extends, implements, {, members, } }   // expression `new class(...) {...}`
Extends { extends: Keyword, types: TokenSeparatedSequence<Identifier> }       // inheritance.rs
Implements { implements: Keyword, types: TokenSeparatedSequence<Identifier> }
```
Class `modifiers` carry `abstract`/`final`/`readonly` (see §10.4).

### 10.2 `enum ClassLikeMember<'arena>` — `class_like/member.rs`

| Variant | Payload (file) |
|---|---|
| `TraitUse(TraitUse)` | `use TraitA, TraitB { ... }` — `trait_use.rs`; adaptations: `insteadof`/`as` aliasing. |
| `Constant(ClassLikeConstant)` | `const X = 1;` — `constant.rs`; `{attribute_lists, modifiers, const, hint: Option<Hint>, items: TokenSeparatedSequence<ClassLikeConstantItem{name,equals,value}>, terminator}`. |
| `Property(Property)` | see §10.3 — `property.rs`. |
| `EnumCase(EnumCase)` | `case Foo;` / `case Foo = 1;` — `enum_case.rs`; `item: EnumCaseItem::{Unit{name}, Backed{name,equals,value}}`. |
| `Method(Method)` | see §10.5 — `method.rs`. |
Helpers: `is_trait_use/is_constant/is_property/is_enum_case/is_method`; and on a member
`Sequence`, trait `ClassLikeMemberSequenceExt` (`contains_methods()`, etc.).

### 10.3 `enum Property<'arena>` — `class_like/property.rs`

- `Plain(PlainProperty{attribute_lists, modifiers, var: Option<Keyword>, hint: Option<Hint>,
  items: TokenSeparatedSequence<PropertyItem>, terminator})` — `public int $x = 1, $y;`.
- `Hooked(HookedProperty{.., item: PropertyItem, hook_list: PropertyHookList})` — property
  hooks `public $x { get {...} set {...} }` (8.4+).
`PropertyItem::{Abstract(PropertyAbstractItem{variable}), Concrete(PropertyConcreteItem{variable,equals,value})}`.
`Property::variables()`, `first_variable()`, `modifiers()`, `hint()`, `var()`.
Hooks (`PropertyHook`, `PropertyHookBody::{Abstract, Concrete{Block|Expression}}`) are
phase-7+ / likely deferred.

### 10.4 `enum Modifier<'arena>` — `modifier.rs`

`Static`, `Final`, `Abstract`, `Readonly`, `Public`, `PublicSet`, `Protected`,
`ProtectedSet`, `Private`, `PrivateSet` (each `(Keyword)`). The `*Set` variants are
asymmetric write-visibility (`private(set)` etc., 8.4). Helpers: `is_visibility/
is_read_visibility/is_write_visibility/is_static/is_final/is_abstract/is_readonly/…`,
`get_keyword()`. Sequence trait `ModifierSequenceExt` gives `contains_static()`,
`get_first_visibility()`, etc.

### 10.5 `Method` — `class_like/method.rs`

```
Method { attribute_lists, modifiers: Sequence<Modifier>, function: Keyword,
         ampersand: Option<Span>, name: LocalIdentifier,
         parameter_list, return_type_hint, body: MethodBody }
MethodBody::Abstract(MethodAbstractBody{terminator})   // `function f();`
MethodBody::Concrete(Block)                             // `function f() { ... }`
```
`is_abstract()`, `is_static()`, `has_promoted_properties()`. Constructor promotion:
params whose `modifiers` are non-empty declare+assign properties (`__construct`).

### 10.6 `enum Access<'arena>` — `access.rs`

| Variant | Payload | |
|---|---|---|
| `Property(PropertyAccess{object, arrow, property: ClassLikeMemberSelector})` | `$o->p` | |
| `NullSafeProperty(NullSafePropertyAccess{object, question_mark_arrow, property})` | `$o?->p` | null-short-circuits |
| `StaticProperty(StaticPropertyAccess{class, double_colon, property: Variable})` | `C::$p` | property is a `Variable` |
| `ClassConstant(ClassConstantAccess{class, double_colon, constant: ClassLikeConstantSelector})` | `C::K` / `C::class` | `C::class` is the special class-name constant |

### 10.7 Member selectors — `class_like/member.rs`

`enum ClassLikeMemberSelector`: `Identifier(LocalIdentifier)` (`->name`),
`Variable(Variable)` (`->$name` dynamic), `Expression(ClassLikeMemberExpressionSelector{left_brace, expression, right_brace})` (`->{expr}`),
`Missing(Span)` (parse recovery — treat as error).
`enum ClassLikeConstantSelector`: `Identifier`, `Expression`, `Missing`.

---

## 11. Namespace, use, declare, constant — `namespace.rs`, `use.rs`, `declare.rs`, `constant.rs`

```
Namespace { namespace, name: Option<Identifier>, body: NamespaceBody }
NamespaceBody::Implicit(NamespaceImplicitBody{terminator, statements: Sequence<Statement>}) // `namespace Foo; ...`
NamespaceBody::BraceDelimited(Block)                                                        // `namespace Foo { ... }`
// Namespace::statements() returns the inner Sequence either way.
```
```
Use { use, items: UseItems, terminator }
UseItems::{ Sequence(UseItemSequence), TypedSequence(TypedUseItemSequence),
            TypedList(TypedUseItemList), MixedList(MixedUseItemList) }
UseType::{Function(Keyword), Const(Keyword)}   // `use function`, `use const`
UseItem { name: Identifier, alias: Option<UseItemAlias{as, identifier}> }
```
Name-resolution only; no runtime behavior (phase 7 symbol tables).
```
Declare { declare, (, items: TokenSeparatedSequence<DeclareItem{name,equal,value}>, ),
          body: DeclareBody::{Statement(&Statement), ColonDelimited(..enddeclare..)} }
// strict_types=1 affects scalar type juggling in calls; ticks/encoding mostly no-op.
Constant { attribute_lists, const, items: TokenSeparatedSequence<ConstantItem{name,equals,value}>, terminator }
```

## 12. Terminators, tags, inline HTML — `terminator.rs`, `tag.rs`, `inline.rs`

`enum Terminator<'arena>` (`Copy`): `Semicolon(Span)` `;`,
`ClosingTag(ClosingTag)` `?>`, `TagPair(ClosingTag, OpeningTag)` `?> ... <?php`,
`Missing(Span)` (recovery / last statement before EOF). `is_semicolon()`,
`is_closing_tag()`. **The interpreter ignores terminators for control flow**, but note:
a `?>` immediately followed by a single newline swallows that newline (PHP rule) —
relevant only if you reconstruct output from `Inline` chunks (mago already splits inline
text into `Statement::Inline`, so just echo `Inline.value` verbatim).

Opening/closing tags surface as `Statement::OpeningTag` / `Statement::ClosingTag` AND
inside `Terminator`. Raw HTML/text between/around tags is `Statement::Inline`
(`InlineKind::Text` → print; `InlineKind::Shebang` → do not print).

`Statement::terminates_scripting()` tells you whether a statement ends in `?>` (script
goes back to inline-HTML mode) — informational; mago already produced the following
`Inline` statements.

---

## 13. High-risk constructs to explicitly NOT support

| Construct | AST node | Action |
|---|---|---|
| `eval(...)` | `Expression::Construct(Construct::Eval(EvalConstruct))` | **Refuse** — raise a "not supported" runtime error. |
| Backtick shell exec `` `...` `` | `Expression::CompositeString(CompositeString::ShellExecute(..))` | **Refuse** — never spawn a shell. |
| `exec`/`shell_exec`/`system`/`passthru`/`proc_open`/`popen` | `Expression::Call(Call::Function{function: Identifier(...)})` (builtin, not a distinct AST node) | Gate in the **builtin registry**, not the AST; do not register them. |
| `extract()` / `compact()` (dynamic scope writes) | `Call::Function` with those names | Gate in builtin registry (no dedicated node); `extract` especially — refuse or sandbox. |
| `create_function()` (legacy eval) | `Call::Function` named `create_function` | Gate in builtin registry — refuse (it is eval-based; removed in PHP 8 anyway). |
| `assert()` with string arg | `Call::Function` named `assert` | Builtin registry — treat string arg as inert, never eval. |
| `include`/`require`(`_once`) | `Construct::{Include,IncludeOnce,Require,RequireOnce}` | Deferred; when added, sandbox file access. |
| `(unset)` cast | `UnaryPrefixOperator::UnsetCast` | Raise fatal (PHP 8.5 behavior); do not perform. |

Note: `eval`, backticks, and `(unset)` have **dedicated AST nodes** and can be refused
structurally in the interpreter's match arms. `extract`/`compact`/`create_function`/
`exec`-family are ordinary function calls with no special node — they must be excluded
at the **builtin function registry** layer.

---

## 14. Phase → node checklist (for match-arm planning)

- **Phase 1 (output):** `Statement::{OpeningTag, ClosingTag, Inline, Echo, EchoTag, Noop}`,
  `Construct::Print`, `Expression::Literal(String)`, basic `Declare`.
- **Phase 2 (scalars & expressions):** all `Literal`, `CompositeString::{Interpolated,
  Document}`, `Binary`, `UnaryPrefix` (ops+casts), `UnaryPostfix`, `Parenthesized`,
  `Conditional`, `Assignment` (`=` and compound), `Variable::Direct`, `ConstantAccess`,
  `MagicConstant`, `Pipe`.
- **Phase 3 (control flow):** `If`, `Switch`, `Match`, `While`, `DoWhile`, `For`,
  `Break`, `Continue`, `Return`, `Throw`, `Try`, `Block`, `Goto`/`Label` (optional).
- **Phase 4 (arrays):** `Array`, `LegacyArray`, `List`, `ArrayAccess`, `ArrayAppend`,
  `ArrayElement::*`, `Foreach`, `Construct::{Isset, Empty}`, `Unset`,
  `Variable::{Indirect, Nested}`.
- **Phase 5 (functions):** `Statement::Function`, `Closure`, `ArrowFunction`,
  `Call::{Function}`, `ArgumentList`/`Argument::*`, `FunctionLikeParameter`,
  `Static` (statics), `Global`, `PartialApplication` (FCC), `Yield` (optional), `Constant`.
- **Phase 6 (references):** `UnaryPrefixOperator::Reference`, `=&`, by-ref params/foreach/
  closure `use (&)`, `Global`, by-ref array elements.
- **Phase 7 (OOP):** `Class`/`Interface`/`Trait`/`Enum`/`AnonymousClass`,
  `ClassLikeMember::*`, `Method`, `Property`, `Modifier`, `Access::*`, `Call::{Method,
  NullSafeMethod, StaticMethod}`, `Instantiation`, `Clone`, `Self_`/`Static`/`Parent`,
  selectors, `Namespace`/`Use` (resolution), `Hint`/type checks (optional enforcement).
