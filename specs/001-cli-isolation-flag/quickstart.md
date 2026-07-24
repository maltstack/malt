# Quickstart: Validate the CLI Isolation Flag

## Prerequisites

- Run from the repository root.
- Have a locally buildable Rust workspace.
- Use an isolated local daemon endpoint. Do not point these checks at a session
  that contains work you need to preserve.

## Focused Automated Validation

Run the CLI crate tests:

```powershell
cargo test -p malt-bin
```

The tests must prove the four accepted lower-case values, name-plus-tier
parsing, the no-option default path, invalid/missing option rejection, request
payload mapping, successful result formatting, and mismatch/error handling.

Run the workspace regression suite before completing implementation:

```powershell
cargo test --workspace
```

## Manual End-to-End Check

1. Build the command and start an isolated daemon on an unused port in one
   terminal:

   ```powershell
   cargo build -p malt-bin
   .\target\debug\malt.exe daemon --port 17700
   ```

2. In a second terminal, create a named Bare session through the new command
   contract:

   ```powershell
   .\target\debug\malt.exe --api-addr http://127.0.0.1:17700 new --name isolation-check --isolation bare
   ```

   Expect one `created session` line containing `isolation-check` and `[Bare]`.

3. Verify the created session is listed with Bare isolation:

   ```powershell
   .\target\debug\malt.exe --api-addr http://127.0.0.1:17700 list
   ```

4. On a machine where the daemon can satisfy each higher tier, repeat step 2
   for `restricted`, `capped`, and `contained`; the creation line and list must
   identify the matching title-case tier.

5. Confirm invalid input creates no success result:

   ```powershell
   .\target\debug\malt.exe --api-addr http://127.0.0.1:17700 new --isolation invalid
   ```

   Expect a non-zero exit and command usage identifying the accepted values.

## Boundary of This Check

This validates the CLI request and reported session tier. It does not prove
OS-level containment, the deferred PTY/compat isolation path, or live Windows
Container behavior; those require their own platform-specific evidence.
