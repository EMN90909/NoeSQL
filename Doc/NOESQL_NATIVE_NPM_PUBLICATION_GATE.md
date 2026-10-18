# NoeSQL native relational execution and npm public release gate

**CURRENT STATUS: NOT NATIVE-QUALIFIED; NOT PUBLISHED ON PUBLIC NPM.**

## Work performed

- Added a fixed native E2E qualification command: `npm run qualify:native-e2e`.
- It validates a Git-pinned Noqeri checkout and compiler source closure via `verify-noqeri-bootstrap.mjs`.
- It requires a pinned native ELF/Mach-O/PE executable SHA-256; JavaScript or shell wrappers fail. An external binary's self-reported identity is not independent proof without compiler provenance.
- It expects the native binary to perform Noqeri-owned schema creation, insert, named query, failure rollback, reopen recovery, checkpoint, and write the actual `NQDB0002` Zig page format.
- It compares named-query output against `Runtime/noesql-cli.mjs` reference results in isolated databases. Current Noqeri compiler tooling cannot yet emit this verified executable, so the gate **correctly fails**.
- Replaced the permissive arbitrary `NOESQL_NATIVE_CONFORMANCE_JSON` release check with this fixed proof-based differential suite.
- Added an offline npm pack-and-install smoke script `npm run test:npm-pack`, which checks package contents for unsafe files and imports the real SQLite-backed NoeSQL API from its tarball.

## Required real native executable protocol

  noesql-native identity --json
  noesql-native run <new-database.nqdb> <source.nqd>
  noesql-native query <database.nqdb> <named-query>
  noesql-native verify <database.nqdb>
  noesql-native checkpoint <database.nqdb>

Identity JSON must contain `engine:"noqeri-zig-relational"`, `native:true`, `storage_format:"NQDB0002"`, and `noqeri_compiler_sha256` matching the independently verified Stage-2 compiler hash. Runtime must truly call Zig `noesql_zig_page_*` from Noqeri-generated machine code and implement authorization/typed relational semantics; simply printing identity fields is insufficient.

## Commands

```sh
npm test
npm run test:npm-pack
npm run qualify:native-e2e     # expected FAIL without a true compiled Noqeri native binary
npm run qualify:zig            # expected FAIL while cross-platform and durability gates remain
```

## npm first publication

The source repository remains private. The package manifest is now marked publishable, but the prepublishOnly hook denies release until the complete compiler and native qualification succeeds. Public `npm publish` is not possible without authorized npm identity and explicit publication approval. We have neither a connected npm publish service nor an authenticated npm publish token here. A local tarball install is **not** public release verification.

After all native and security gates pass, the repository owner must:
1. Choose and check in the correct license and verify no proprietary/secret files enter the tarball.
2. Verify ownership/availability of the `noesql` npm package coordinate.
3. Authenticate using npm's trusted publisher OIDC or a supported 2FA-backed release. No long-lived npm token should be committed to source.
4. Build/publish the exact reviewed archive. Verify public metadata, SHA-512 integrity, install in a fresh online environment, and run real SQL read/write/restart tests.
5. Only then change the website's status from 'future npm command' to 'published'.

**Do not reclassify the experimental SQLite adapter as a production Zig relational executor.**
