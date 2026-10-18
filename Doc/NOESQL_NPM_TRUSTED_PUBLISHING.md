# npm public release preparation and trusted publishing

NoeSQL 0.7.6 has package metadata for public npm, GPL-3.0-only licensing aligned with the Noqeri parent repository, and a verbatim GPLv3 LICENSE. Third-party notices and ownership must still receive maintainer legal review.

The prepublishOnly hook scripts/release-preflight.mjs requires approved SHA, release sign-off, license sign-off, Noqeri stage0/provenance, native binary and Zig E2E differential SQL test, Zig qualification, Node tests and locally packed tarball installation. A successful source review cannot substitute for native qualification.

The GitHub workflow npm-publish.yml is manual-only, runs under the npm-public-release GitHub Environment, uses npm 11 with GitHub OIDC (id-token: write), and publishes exclusively to the next dist-tag. Configure reviewers for the release environment. The package maintainer must own or claim noesql at npmjs.com and authorize trusted publishing with GitHub owner EMN90909, repository NoeSQL, workflow npm-publish.yml, and environment npm-public-release. An initial publication may require separately authenticated registration before OIDC enrollment.

The workflow cannot create a missing trusted compiler seed, Noqeri compilation proof, or native executable. Those artifacts are not currently installed on the GitHub runner and the release gate correctly blocks on them. Because the GitHub repository is private, npm's public-source provenance may remain unavailable even if publication is authorized.

No authenticated npm publisher is connected to the assistant. A public npm package, active trusted publisher, or successful npm install cannot be claimed yet.
