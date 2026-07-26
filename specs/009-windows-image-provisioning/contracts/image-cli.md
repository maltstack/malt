# CLI contract: `malt image`

The first image-management surface is an operator CLI. Commands call the
daemon, which uses the authenticated elevated helper; they do not read or
write HCS paths directly.

## Commands

```text
malt image provision <registry-reference>
malt image list
malt image inspect <image-id-or-manifest-digest>
malt image remove <image-id-or-manifest-digest>
```

### `provision`

Input is a public OCI registry reference. On success, stdout contains:

```text
image:       maltimg:<opaque-id>
manifest:    sha256:<64-hex>
platform:    windows/amd64 (os.version <version>)
readiness:   ready | unavailable
assessment:  <sanitized reason or verified evidence>
```

The command refuses a mutable/ambiguous selection until it has selected and
recorded an immutable manifest digest. A failure returns nonzero, names the
stage (`resolve`, `verify`, `prepare`, or `assess`), and exposes neither an HCS
storage document nor an internal filesystem path.

### `list`

Displays one row per helper-owned provisioned image: opaque ID, manifest
digest, selected platform, readiness, active dependent session count, and
last-assessed time. It displays no user-controlled cached directories.

### `inspect`

Returns the same immutable identity plus ordered descriptor digests, readiness
reason/evidence basis, and active dependent session identifiers. A manifest
digest resolves only when it names exactly one helper-owned record.

### `remove`

Removes an unused helper-owned record and all artifacts marked as belonging to
that record. If any active session references it, it exits nonzero and lists
the dependent session IDs. Absence is idempotently reported as `not found`,
not as successful deletion of an arbitrary directory.

## Session selection

The existing `malt new --isolation contained` command gains an explicit image
selector:

```text
malt new --isolation contained --image <image-id-or-manifest-digest>
```

`--image` is required when more than one ready image exists. If exactly one
ready image exists, the daemon may select it and must report the selected
manifest digest in session creation output. `required` refuses unavailable or
missing selection; `preferred` preserves the existing visible bare downgrade.

