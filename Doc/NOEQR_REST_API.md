# NoeSQL .noeqr query/schema scripts and HTTP REST

**Status: experimental Linux-native integration, NOT a production-facing network service.**
The REST adapter calls the existing NQDB0003 NoeSQL parser and Rust v3
generation store. It does not add another database engine or Node/JavaScript
dependency. It does NOT implement remote TLS, persistent user authentication,
tenant authorization, query CPU/memory limits or full crash qualification.

## The extensions

- **.noeqr**: plain UTF-8 NoeSQL query, schema, migration or DML source.
- **.noqeri**: Noqeri application/compiler source, not a database query script.
- **.noedb**: existing durable NoeSQL database data file; its format does not change.
- **.noe**: retired NoeSQL script extension, rejected by file-based CLI commands.
  Rename or copy reviewed old scripts to .noeqr (not database files).

For a private existing Linux v3 database, run local scripts with:

    noesql check /private/scripts/schema.noeqr
    noesql v3-create /private/data/app.noedb /private/scripts/schema.noeqr
    noesql query-file /private/data/app.noedb /private/scripts/list.noeqr
    noesql schema-file /private/data/app.noedb /private/scripts/migration.noeqr
    noesql run /private/data/app.noedb /private/scripts/mixed-batch.noeqr

query-file is read-only. schema-file permits schema/index/named-query
declarations only. run is a trusted administrator's mixed statement executor.
The REST endpoint deliberately **does not** permit schema migrations.

## Start the loopback HTTP service

Requires an existing valid private NQDB0003 store, optional owner-private
scripts directory (0700) and 0600 script files, and a strong secret.

    export NOESQL_API_TOKEN="$(python3 -c 'import secrets;print(secrets.token_hex(32))')"
    noesql serve /private/data/app.noedb 8087 --scripts-root /private/scripts

The HTTP listener binds only 127.0.0.1, never a public network address. Do
not expose this endpoint outside a trusted local application. All routes except
GET /health require the token. Standard clients send Authorization:
Bearer [secret]. No CORS or cookie authentication is enabled; browser Origin
requests are rejected.

| Method / path | Purpose |
| --- | --- |
| GET /health | Process liveness only, no disk readiness claim |
| GET /v1/status | Verify v3 head; generation, bytes and page count |
| POST /v1/query | Parameterized read-only GET queries |
| POST /v1/execute | Parameterized add/set/delete DML with durable v3 commit |

For POST, use Content-Type: application/json and one of:

    {"source":"people get id,name where name == $name","params":{"name":"Ada"}}
    {"script":"list.noeqr","params":{"name":"Ada"}}

Script filenames must be simple ASCII .noeqr basenames inside the private
--scripts-root. No absolute paths, separators, traversal, arbitrary files
or script symlinks. The existing native session JSON parser handles typed
parameters, int64, booleans, null, strings, exact decimals and tagged scalar
values without source interpolation. Response format for a read:
{"rows":[{"name":"Ada"}],"count":1}; for a write: {"affected":1}.

Example with standard HTTP Authorization:

    curl -fsS http://127.0.0.1:8087/v1/query \
      -H "Authorization: Bearer $NOESQL_API_TOKEN" \
      -H "Content-Type: application/json" \
      --data '{"source":"people get id,name where name == $name",
               "params":{"name":"Ada"}}'

**Native Noqeri compatibility:** The Noqeri main revision scanned was
3c00224accb6230d0148dd90f2fe21ef9cc372f0.
Lib/std/http/client.noqeri exposes http_client_json_post, but cannot
supply arbitrary Authorization headers. The NoeSQL loopback-only REST
adapter therefore also accepts a strong token in the POST JSON envelope:

    {"token":"64-hex-character secret from local config",
     "script":"list.noeqr","params":{"name":"Ada"}}

Use this entire JSON payload with
http_client_json_post("http://127.0.0.1:8087/v1/query", body, 30000 as i64),
then http_client_status, http_client_body, and http_client_close.
See examples/noeqr_http_client.noqeri. Do not commit secrets, send them to a
browser or transmit them over unencrypted non-loopback HTTP. Standard bearer
headers are preferred when the client can set them.

HTTP/1.1 restrictions: one request per TCP connection, Content-Length only,
no chunking/compression/pipelining, 16 KiB headers, 128 KiB JSON body,
65,536-byte source, 32 bindings, 1,024 rows, 1 MiB serialized response,
32 concurrent clients, five-second socket I/O timeout.
Errors return a JSON error code and message. HTTP 409 = conflict.
HTTP 503 commit_uncertain means a write may already be published: **check
the generation before retrying**.

## Test without GitHub Actions

    make test-noeqr-rest-local

This builds and runs Rust parser/server tests and a real separate-process
REST integration fixture: v3 schema creation, .noeqr CLI, HTTP bearer/body
tokens, prepared values including injection-like text, read-only enforcement,
DDL denial, private script lookup and persisted generations.
No GitHub Actions, Node, JavaScript or mock database implementation.

The Noqeri noesqlHost* native source-level extern link is separate from
this REST integration. The full production exit gates (power loss, ENOSPC,
multi-client soak, resource controls, TLS/public AuthN/AuthZ, real Noqeri
frontend+host linking) remain open. See
Doc/PRODUCTION_ENGINE_EXIT_CRITERIA_2026-11-07.md.
