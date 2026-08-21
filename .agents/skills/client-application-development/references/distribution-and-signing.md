# Distribution And Signing

Use this generic reference when release, immutable binary delivery, signing, or source admission changes the result. Resolve the actual host, prefix, channel, and publisher from the project's ProductContract, Runbook, or workspace-owned distribution policy.

## Distribution Modes

Classify delivery as `self-use`, `internal`, `controlled`, or explicit `production` promotion. A local build, candidate, tag, OSS object, or Git push does not by itself promote a client to production or authorize Deployment.

Keep one canonical version owner per independently released application, installer, launcher, capability, or payload. Freeze the source commit, bytes, object key, size, digest, platform, architecture, and provenance before publication.

## Immutable Source

Use the configured immutable source and exact object keys. Do not discover binaries through directory listings, filename guesses, mutable latest endpoints, a second mirror, or an undeclared registry/GitHub fallback. Upload immutable assets first, read them back through the public path, and update one mutable pointer only after the complete closure is readable.

Retries reuse the frozen candidate and verified objects. They do not rebuild or overwrite a same-version immutable object, advance a pointer over an incomplete closure, or substitute another binary origin.

## Integrity And Publisher Identity

Verify byte count, SHA-256, platform, architecture, safe paths, compatibility, and any component-specific provenance before activation. SHA-256 proves integrity, not publisher identity.

Use publisher signing, notarization, store provenance, or another channel-specific admission only when the product/channel contract requires it. Missing self-use publisher credentials do not block local Development when no production admission requires them; third-party and platform-owned components retain their own upstream provenance.

## Deployment Boundary

Publication to a named target remains a separate Deployment task. Deployment requires its own artifact, target, admission evidence, authorization, rollback, and post-deployment checks.
