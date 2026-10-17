# ADR 0003: Parser, Policy, And ExecutionPlan Boundary

Date: 2026-06-08

## Status

Accepted

## Context

Site parsers, protocol parsers, and downloaders previously shared some strategy decisions, especially output naming and source-specific URL suffix extraction. That made behavior harder to test and encouraged site-specific assumptions to leak into protocol or execution code.

## Decision

Keep these boundaries:

- Site parsers identify site facts and candidate resources.
- Protocol parsers convert resolved resources into an `ExecutionPlan`.
- Policy modules own reusable strategy decisions such as output naming.
- `ExecutionPlan` is the contract between parsing and execution.
- Downloaders execute a plan and return the final output path; they should not own naming policy unless the policy is local to the transport implementation.

The first accepted policy module is `src/parsers/policy/output_naming.rs`.

## Consequences

- Bilibili, X/Twitter, YouTube, and HTTP naming behavior is testable without running downloads.
- Future quality, retry, source identity, and rate limit policies should follow the same boundary.
- Parser and downloader code should not accumulate new ad hoc filename rules.
