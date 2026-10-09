# Security

The unpublished 0.2 series receives fixes on `main`. This project has no guaranteed response time or long-term support commitment.

For a suspected security problem, contact the repository owner privately through an existing authenticated channel. GitHub private vulnerability reporting may be used if it is enabled for the repository. Do not open a public issue containing an exploit, credentials or confidential acquisition data.

Provide the package revision, operating system, Rust version, resource-limit settings and a minimal synthetic reproducer when possible. Describe whether a problem involves unexpected writes, unbounded resource use, a panic or incorrect decoded data.

This reader targets completed local acquisitions. It rejects nonempty SQLite journals and opens metadata read-only. These checks do not create a snapshot or protect against another process modifying the acquisition concurrently.

Frame and metadata limits reduce accidental resource exhaustion. The pinned prerelease Turso engine can allocate during schema preparation or while constructing a SQLite value before this library checks materialized output. The query deadline and VM budget do not impose a hard process memory limit. Isolate processing of hostile databases and apply operating-system resource limits where needed.

No publication or deployment workflow is enabled. CI requires read-only repository permissions and does not receive a crates.io publishing token.
