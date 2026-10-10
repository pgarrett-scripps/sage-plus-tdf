# Security

## Supported versions

Only the latest published release of `sage-plus-tdf` receives fixes, on `main`. This project has no guaranteed response time or long-term support commitment.

## Reporting a vulnerability

Report suspected security problems privately through GitHub's [private vulnerability reporting](https://github.com/pgarrett-scripps/sage-plus-tdf/security/advisories/new) for this repository. Do not open a public issue containing an exploit, credentials or confidential acquisition data. If private reporting is unavailable, open a public issue that only asks for a private contact, without details.

Provide the crate version or revision, operating system, Rust version, resource-limit settings and a minimal synthetic reproducer when possible. Describe whether a problem involves unexpected writes, unbounded resource use, a panic or incorrect decoded data.

## Trust boundary

This reader targets completed local acquisitions. It rejects nonempty SQLite journals and opens metadata read-only. These checks do not create a snapshot or protect against another process modifying the acquisition concurrently.

Frame and metadata limits reduce accidental resource exhaustion. The Turso SQLite engine can allocate during schema preparation or while constructing a SQLite value before this library checks materialized output. The query deadline and VM budget do not impose a hard process memory limit. Isolate processing of hostile databases and apply operating-system resource limits where needed.

## Release pipeline

CI runs with read-only repository permissions. Only the tag-triggered release workflow can publish, through crates.io Trusted Publishing (short-lived OIDC tokens) or a repository secret used for the first release; see [RELEASING.md](RELEASING.md).
