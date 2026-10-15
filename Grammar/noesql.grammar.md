# NoeSQL experimental reference grammar (2026-10-15)

**Status:** executable *subset* for the reference implementation at `Runtime/noesql-reference.mjs`. Not the final production language grammar. The independent Noqeri scanner at `Database/noesql_lexer.nqr` recognizes the same lexical categories but has not passed Stage-2 binary verification. The broader NoeSQL roadmap in `Doc/NOESQL_IMPLEMENTATION_STATUS.md` is not part of this grammar's currently executable semantics.

A NoeSQL script is `.noe`; `.nqr` remains Noqeri source. Experimental `.nqd` stores database *data*, not executable Noqeri source. `.nqo` remains compiled Noqeri object/web output. NoeSQL statements do not become Noqeri `.nqr` grammar productions until an actual compiler integration passes conformance.

```ebnf
unit          = { separator | declaration, boundary | statement, boundary } ;
declaration   = db | table | index | query | alter ;
db            = "db", identifier ;
table         = "table", identifier, "{", { separator | field, fieldBoundary }, "}" ;
field         = identifier, ":", type, [ "?" ], { modifier }, [ "=", expression ] ;
type          = "int" | "text" | "bool" | ("ref", identifier) ;
alter         = "alter", identifier, "add", "{", { separator | field, fieldBoundary }, "}" ;
modifier      = "required" | "unique" | "key" | "auto" ;
index         = "index", identifier, "on", identifier,
                "(", identifier, { ",", identifier }, ")" ;
query         = "query", identifier, "(", [parameters], ")", "{",
                {separator}, select, {separator}, "}" ;
parameters    = parameter, { ",", parameter } ;
parameter     = identifier, ":", ("int" | "text" | "bool"), ["=", expression] ;
statement     = insert | select | update | delete | atomic ;
insert        = identifier, "add", object ;
select        = identifier, "get", ("*" | projection, {",", projection}),
                ["join", identifier, "on", expression],
                ["where", expression],
                ["group", qualified, {",", qualified}],
                ["sort", qualified], ["skip",expression], ["take",expression] ;
projection    = qualified | aggregate ;
aggregate     = ("count" | "sum" | "min" | "max"), "(",
                ["*" | qualified], ")", ["as", identifier] ;
qualified     = identifier, [".", identifier] ;
update        = identifier, "where", expression, "set", assignments ;
delete        = identifier, "where", expression, "delete" ;
atomic        = "atomic", "{", { separator | statement, boundary }, "}" ;
object        = "{", { separator }, [ assignment, { "," | separator, assignment } ], "}" ;
assignments   = assignment, { ",", assignment } ;
assignment    = identifier, (":" | "="), expression ;
names         = identifier, { ",", identifier } ;
expression    = logicalOr ;
logicalOr     = logicalAnd, {"||", logicalAnd} ;
logicalAnd    = equality, {"&&", equality} ;
equality      = comparison, {("==" | "!="), comparison} ;
comparison    = additive, {("<" | "<=" | ">" | ">="), additive} ;
additive      = multiply, {("+" | "-"), multiply} ;
multiply      = unary, {("*" | "/"), unary} ;
unary         = ("!" | "-"), unary | primary ;
primary       = integer | string | "true" | "false" | "null" |
                identifier | "$", identifier | "(", expression, ")" ;
identifier    = (ASCII_LETTER | "_"), {ASCII_LETTER | DIGIT | "_"} ;
integer       = DIGIT, {DIGIT} ;
string        = '"', { JSON_STRING_CHARACTER_OR_ESCAPE }, '"' ;
separator     = LF | ";" ;
boundary      = separator, {separator} | EOF | "}" ;
fieldBoundary = ("," | separator), {separator} | "}" ;
```

**Lexical contract:** ASCII identifiers are case-sensitive. `__proto__`, `prototype` and `constructor` are reserved in the reference interpreter. A `//` comment ends at LF; `/* ... */` comments are non-nested and must terminate. The reference lexer emits absolute character offsets and 1-based line/column diagnostics. CR and horizontal whitespace are otherwise insignificant. Inside an expression, LF and semicolon are statement boundaries (not implicit expression continuation). JSON double-quoted string escapes are accepted. Adjacent statements must have a boundary; commas delimit object fields and assignment lists.

**Execution contract:** `get` is mandatory for retrieval, and `where` is mandatory for `set` and `delete`. `get` returns selected rows; `add` returns the inserted row and an affected count; `set` and `delete` return affected counts. Equality is strictly typed, truth conditions must be booleans, and null comparisons evaluate to unknown (not true). `&&` and `||` short-circuit and follow three-valued logic. Arithmetic only accepts safe integers and checks overflow. The reference rejects unsupported scalar types rather than converting money to float.

**Current restrictions:** one integer primary key per table; only scalar `int`, `bool`, `text` and integer-key `ref` types; single-column sort; indexes currently have schema metadata only (no physical index seek); no outer/self joins, HAVING, DISTINCT, GROUPING SETS, date/decimal, explicit `is null`, destructive schema migrations, byte storage or typed Noqeri binding generation. **A bounded reference-only subset of inner `join`, `group`, `count`, `sum`, `min`, `max` and additive `alter` is available in the JavaScript implementation.** These are NOT silently accepted. NoeSQL does not yet parse inline inside a `.nqr` compilation unit.

## Readable relational queries (experimental reference engine only)

```noe
table customers {
  id: int key auto
  name: text required
}
table orders {
  id: int key auto
  customer_id: ref customers required
  amount: int required
}

orders get customers.name, count(*) as orders, sum(amount) as total
  join customers on customer_id == customers.id
  group customers.name
  sort customers.name
```

The parser currently requires each query statement on **one logical line**; the multi-line presentation above is for readability, not directly executable. Use a single line for the `orders get...` statement. Joining a second table exposes its fields under `table.field`; base table fields remain usable without qualification. Grouped projections must be declared in `group` unless they are aggregates. `count()` and `count(*)` count rows; `count(column)` ignores nulls. `sum` accepts integer fields only; empty `sum/min/max` yields null, and sum overflow is rejected.

```noe
alter orders add { fulfilled: bool required = false }
```

Additive migration supports scalar non-key, non-unique fields only. A required field needs a literal default on an existing nonempty table. The reference database schema version increments when the change succeeds. No general migration framework, index rebuild, compatibility guarantee, or native storage upgrade is implied.

Execution is limited to **100,000 matching rows** and **1,000,000 candidate join pairs** per query. This is a reference safety budget rather than an optimizer guarantee. Native Noqeri compilation and durable isolation remain unverified.
