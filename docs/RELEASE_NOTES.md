# Release Notes

## 2026-07-26 — Fail-Closed Session Isolation

Breaking protocol and API change: `SessionInfo.isolation` is now an
`IsolationStatus` object (`effective`, `requested`, `basis`, `mechanism`, and
`detail`) rather than a bare `IsolationTier`. Consumers must read the
effective tier and basis instead of treating a request as proof of containment.

Naming a non-bare tier now defaults to `required`. Calls that historically
received a silently uncontained session can therefore fail with
`isolation_unavailable`. Pass `isolation_policy=preferred` (or
`malt new --isolation-policy preferred`) to explicitly accept a visible
downgrade; use `disabled` to request the bare baseline deliberately.
