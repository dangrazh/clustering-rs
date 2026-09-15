# Multi-user implementation status and acceptance evidence

Date: 2026-09-14. Scope: [implementation plan](multi-user-implementation-plan-20260913.md), applying the agreed [architecture](multi-user-architecture-20260913.md) and [requirements](multi-user-requirements-20260913.md).

The application implementation is available in the working tree, with local Rust, browser, recovery and workload verification. It has not been deployed or accepted for production. M8/G3 and the company-filesystem portion of G2 remain open; no stakeholder exception is assumed. Entra registration values are still unavailable, as previously confirmed by the user.

## Delivered implementation

| Plan area | Implemented result and evidence |
| --- | --- |
| M0 / P00–P02 | Baseline preserved; typed runtime limits, command/version contracts and operator configuration documented. Isolated identities support development while company Entra inputs remain pending. |
| M1 / P03–P06 | Pinned embedded Turso 0.7.2, actual schema version 1, MVCC/FULL durability, four-writer admission, lifecycle/parent coordination, command receipts and transactional outbox. Production storage tests cover concurrent edits/claims, stale versions, ownership, restart and forced process death. |
| M2 / P07–P09 | Entra code flow with PKCE/state/nonce/signature checks; opaque sessions and CSRF; owner-only temporary data; checksummed immutable artifacts; idempotent unique named central save; shared catalog and reopen. Actual HTTP tests use two authenticated identities. |
| M3 / P10–P12 | Separate label/workflow/reviewer/comment versions, existing workflow rules, owner-only comments, reviewer directory and history. Conflict drafts survive; transport retries and same-user sign-in renewal preserve command IDs. Replayed deleted-comment creation does not resurrect the comment. |
| M4 / P13–P16 | Durable ordered publication, reconnecting SSE, obsolete-snapshot protection, separate autosaved personal views, transient presence, rename/archive/restore and reserved names. Browser tests cover independent views and archived dirty forms. |
| M5 / P17–P19 | Durable FIFO queue, one supervised child, bounded thread count, timeout/cancel/failure handling, restricted child environment, restart reconciliation, My jobs and displayed retention. Actual child process output reopens through the application. |
| M6 / P20–P22 | Legacy schema 1/2 import; schema-3 portable export with reviewer/audit records; imported comments read-only and reviewers unconfirmed; consistent Excel export paths. `.icar` service routes removed. |
| M7 / P23–P25 | Non-root Linux image definition, instance exclusion, stopped backup/restore with checksummed manifest, startup cleanup, healthcheck, runtime metrics and operator runbook. Populated local backup/restore passes. Company image/mount rehearsal remains pending. |
| M8 / P26–P28 | Local functional and synthetic workload evidence below. Company Entra/HTTPS/container acceptance, representative mixed workloads, remaining deployment fault rehearsals and rollout are not complete. |

Concrete module/HTTP contracts are documented in [current web architecture](web-architecture.md). Configuration, backup commands, upgrade procedure and release prerequisites are in [operations](multi-user-operations-20260914.md).

## Correctness evidence

- Rust regression suite: 35 library tests and 3 session integration tests pass. Explicit workloads and browser/process fixtures are intentionally excluded from the ordinary run; the internal crash child must only be launched by its parent test.
- JavaScript unit suite: 9 tests pass.
- Formatting and strict Clippy pass with one explicit allowance for the pre-existing `ProgressReporter::substep` argument count.
- Actual HTTP flow passes: anonymous/CSRF rejection, owner-only import, central save, another user's reopen/edit, server-established actor, stale conflict, portable export and pivot.
- Two Chromium contexts pass: conflicting label drafts and reconsideration, automatic changes, comment attribution/ownership, claiming, personal view reopening, lost-ack retry without duplication, same-account sign-in renewal, refusal to retry as another identity, archive/restore and portable import. This uses opaque fixture sessions against actual handlers, not a production sign-in bypass.
- Actual executable child passes: durable queued input, supervised clustering, published immutable output and reopened result. Queue tests cover ownership, cancellation and queued/running restart reconciliation.
- Forced process death passes: an acknowledged commit survives killing a process with another transaction unfinished; reopening recovers records and publishes outstanding changes. This tests application process failure, not host/storage loss.
- Populated stopped backup/restore passes: shared labels, comments/ownership, audit, personal view, immutable source/result and retained temporary job survive restoration. A live backup is rejected; a tampered database fails manifest verification before restoration.
- Existing workflow transitions, required fields, inheritance/undo, labels, Excel contents, codec integrity and version-1 compatibility remain covered. Concurrent claims yield one assignee. Reviewer changes preserve SME email, action-owner email and ETA; workflow changes preserve the reviewer.

## Local performance evidence

These are synthetic measurements, not production acceptance. Linux results use WSL Ubuntu 22.04, Rust 1.93.1, release optimization and local ext4 temporary storage. The WSL environment exposed approximately 12 logical CPUs and 15.2 GiB RAM; it was not the intended eight-core/64-GiB container. The large fixtures contain generated source values across 30 columns, plus existing synthetic analysis memberships. They do not rerun large-scale clustering.

| Rows | Artifact bytes | Result JSON bytes | Initial artifact + catalog save | Cold open + review read + JSON serialization |
| --- | ---: | ---: | ---: | ---: |
| 20,000 | 12,951,726 | 33,626,734 | 551 ms | 148 ms |
| 150,000 | 97,202,417 | 252,741,314 | 4,399 ms | 1,158 ms |
| 200,000 | 129,610,710 | 337,174,704 | 4,721 ms | 1,435 ms |

The 200,000-row case verifies retained functional capacity. These timings exclude HTTP transfer, browser rendering, real XLSX import and clustering. The two workload tests ran concurrently; no unsupported precision about an isolated production workload is inferred.

The storage workload represented 500 directory users and 1,000 saved analyses, with 50 active reviewers split across five analyses (10 each). Each reviewer submitted 20 comments and read snapshots, using four active writers. All 1,000 comments and 2,000 expected catalog/edit deliveries were present after publication. Command latency including admission and commit: p50 69 ms, p95 83 ms, p99 86 ms, worst 89 ms. These are storage-command timings, not HTTP or SSE visibility timings.

A separate Windows debug-server/Chromium 153.0.8010.12 check reopened 150,000 rows and 30 columns through actual HTTP and rendered the result tree in **25,323 ms**, with approximately **588,000,000 bytes of JavaScript heap** reported by Chromium. This was one local HTTP browser with generated field values; initial fixture creation was excluded. The browser test asserts readiness below 30 seconds. This is evidence that the existing full-result approach can work locally, not proof of the company-network or simultaneous-open target.

Run entry points:

```sh
cargo test --locked
npm test
cargo fmt --check
cargo clippy --all-targets -- -D warnings -A clippy::too_many_arguments
cargo build --locked
cargo test --lib supervised_child_publishes_reopenable_result -- --ignored --nocapture
cargo test --lib browser_collaboration_acceptance -- --ignored --nocapture
cargo test --lib large_browser_reopen_acceptance -- --ignored --nocapture
cargo test --release --lib acceptance_tests -- --ignored --nocapture
```

Browser commands require the installed Playwright dependency and Chromium; see operations for setup. Windows linking intermittently returned LNK1104 on existing test executables; unchanged retries completed successfully. No source workaround or suppressed test failure was used.

## Acceptance boundaries and outstanding release work

| Boundary | Remaining evidence or decision |
| --- | --- |
| Entra / AC-18 | Tenant/client registration, HTTPS callback and injected secret; actual sign-in/token exchange/key rotation/logout/expiry test. Confirm guest eligibility, session lifetime and access-removal policy with IT. Signed token fixtures and application-session tests are already present. |
| Container / G2 | Build and run the supplied Dockerfile on the company host, verify UID/mount permissions and exclusive instance behavior, then restore a populated backup on the real filesystem. Local Docker image retrieval encountered registry/CDN errors; no successful container build/run is claimed. |
| Timing / AC-04, AC-08, AC-19 / G3 | Finalize measurement method and history volumes; exercise real HTTPS/SSE and representative 20k/150k XLSX files with 50 active users, one clustering child, up to 10 submitted jobs and simultaneous save/open/export. Measure end-to-end save, visibility, presence and browser readiness distributions. |
| Fault rehearsal / P27 | Complete host-level disk-exhaustion, interrupted publication/worker completion, upgrade and ingress/disconnection rehearsals on the staging deployment. Local process-kill, retry, stale-version, backup corruption and restart tests do not substitute for all these scenarios. |
| Operations / P28 | Configure external backup destination/schedule/retention and alerts; agree any RPO/RTO with IT. Preserve current users' in-memory work as downloaded files before the maintenance cutover. Perform reviewer staging acceptance and a compatible-backup rollback rehearsal. |

## Explicit implementation choices and limits

- The Entra fallback was not activated. No unverified-email login or production fixture mode was added.
- History/comments and result data are currently full snapshots. Mutation transactions only update affected records. Large-volume history growth and production network measurements must determine whether pagination is needed before G3; no claim is made that paging was implemented.
- Physical cleanup runs at startup; logical expiry applies continuously. This removes cleanup/publication races but requires maintenance restarts to reclaim expired files. Command receipts and delivery history are retained indefinitely and need disk monitoring.
- Runtime logs expose admission/command latency, retries, publication backlog count, presence views, queue, cache estimate and disk availability. The hosting environment supplies child CPU/RSS and container resource monitoring. Backlog count is not publication age or end-to-end delivery latency.
- Heavy operations default to two concurrent requests and return retryable 503 when capacity is exhausted. Worker timeout defaults to one hour and is configurable. Both are operator-tunable initial limits, not stakeholder-approved production sizing measurements.
- Personal state follows the stable user identity; no shared navigation, per-analysis permissions, permanent deletion, offline recovery, `.icar` replacement or in-place reclustering was introduced.

## 2026-09-15 follow-up: approved email fallback

Email-only allowlist sign-in is now implemented at the user's request. The previously unactivated fallback boundary above is superseded: users supply email only, and the externally maintained file supplies the name. Local startup is available through `start-local.ps1` after creating `data/allowed-users.json`. Entra setup can proceed separately. Added tests cover allowlist validation/reload/removal, stable identity, mapped names, login request protection, logout and the browser collaboration flow starting from the actual email sign-in form. See the operator guide's email allowlist section for configuration and identity-transition behavior.
