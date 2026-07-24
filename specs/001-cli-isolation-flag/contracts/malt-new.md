# Command Contract: `malt new --isolation`

## Syntax

```text
malt new [--name <NAME>] [--isolation <TIER>]
```

`<TIER>` is exactly one of:

| CLI value | Requested session tier | Expected reported tier |
|---|---|---|
| `bare` | `bare` | `Bare` |
| `restricted` | `restricted` | `Restricted` |
| `capped` | `capped` | `Capped` |
| `contained` | `contained` | `Contained` |

## Request Semantics

- With `--isolation`, the command sends the selected lower-case value with the
  optional name to the existing session-creation authority.
- Without `--isolation`, the command preserves the existing request shape and
  relies on the established Bare default.
- `--name` and `--isolation` apply to the same new session regardless of their
  order on the command line.

## Result Semantics

On a matching creation result, standard output includes the session identifier,
name (or `-`), and reported isolation tier:

```text
created session <id> (<name-or-->) [<ReportedTier>]
```

For example:

```text
created session 42 (build) [Capped]
```

The command exits unsuccessfully and prints no successful creation line when:

- the option value is missing or not one of the four listed values;
- the session authority returns a creation error; or
- the returned tier differs from the requested (or default Bare) tier.

For a reported-tier mismatch, the error identifies both the requested and
reported tiers. It does not retry with Bare or delete a remotely created
session.
