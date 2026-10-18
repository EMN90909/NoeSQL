# NoeSQL language usability review (October 2026)

Scores below are subjective editorial judgments about *first-day learning*
for simple CRUD, not objective usability experiments, ecosystem rankings,
or claims that NoeSQL implements every feature shown in other languages.

| Language/API | First-day CRUD readability (10=max) | Why |
| --- | ---: | --- |
| NoeSQL `.noe` reference syntax | 9.0 | `users add`, `users get ... where ...`, concise schemas, few keywords |
| SQLite / standard SQL | 7.5 | Predictable and portable, but `SELECT ... FROM`, DDL and joins introduce boilerplate |
| PostgreSQL / standard SQL | 7.5 | Same basic SQL learnability; richer features increase later complexity |
| MongoDB shell API | 7.0 | Expressive document CRUD, but method chains and nested JSON filters |
| Redis commands | 8.5 | Very short key-value commands, but data modeling/query power differs |
| Firestore SDK APIs | 6.5 | Readable SDK operations, but async APIs, paths, indexes and rules add context |

Example:

```noe
table users {
  id: int key auto
  name: text required
  age: int
}
users add {name:"Amina", age:21}
users get name, age where age >= 18 sort name take 10
```

The readable `.noe` grammar is currently implemented in an experimental JS
reference executor and optional adapters. The C++ native page store does not
parse `.noe` scripts; do not confuse the readability score with native feature
completeness. More rigorous ratings require independent novice usability tests.

Keep ease-of-learning without hiding essential semantics: explicit transactions,
prepared parameters, useful type errors, migration feedback, query-plan display,
index hints, diagnostics and permission errors must stay comprehensible.
