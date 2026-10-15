# NoeSQL experimental reference grammar (2026-10-15)

**Status:** executable *subset* for the reference implementation at `Runtime/noesql-reference.mjs`. Not the final production language grammar. The independent Noqeri scanner at `Database/noesql_lexer.nqr` recognizes the same lexical categories but has not passed Stage-2 binary verification. The broader NoeSQL roadmap in `Doc/NOESQL_IMPLEMENTATION_STATUS.md` is not part of this grammar's currently executable semantics.

A NoeSQL script is `.noe`; `.nqr` remains Noqeri source. Experimental `.nqd` stores database *data*, not executable Noqeri source. `.nqo` remains compiled Noqeri object/web output. NoeSQL statements do not become Noqeri `.nqr` grammar productions until an actual compiler integration passes conformance.

```ebnf
unit          = { separator | declaration, boundary | statement, boundary } ;
declaration   = db | table | index | query ;
db            = "db", identifier ;
table         = "table", identifier, "{", { separator | field, fieldBoundary }, "}" ;
field         = identifier, ":", type, [ "?" ], { modifier }, [ "=", expression ] ;
type          = "int" | "text" | "bool" | "ref", identifier ;
modifier      = "required" | "unique" | "key" | "auto" ;
index         = "index", identifier, "on", identifier,
                "(", identifier, { ",", identifier }, ")" ;
query         = "query", identifier, "(", [parameters], ")", "{",
                {separator}, select, {separator}, "}" ;
parameters    = parameter, { ",", parameter } ;
parameter     = identifier, ":", ("int" | "text" | "bool"), ["=", expression] ;
statement     = insert | select | update | delete | atomic ;
insert        = identifier, "add", object ;
select        = identifier, "get", ("*" | names), ["where",expression],
                ["sort", identifier], ["skip",expression], ["take",expression] ;
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

**Current restrictions:** one integer primary key per table; only scalar `int`, `bool`, `text` and integer-key `ref` types; single-column sort; indexes currently have schema metadata only (no physical index seek); no `group`, aggregates, joins, relationship projection, date/decimal, explicit `is null`, migrations, byte storage or typed Noqeri binding generation. These are NOT silently accepted. NoeSQL does not yet parse inline inside a `.nqr` compilation unit.
