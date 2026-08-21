# Workspace Delivery Defaults

This reference contains workspace-specific distribution policy. It is not a generic client rule and must be loaded only when release, OSS publication, signing, or immutable binary delivery is in scope.

## Binary Origin

Use the fixed `https` endpoint `shared-public-assets.oss-cn-beijing.aliyuncs.com` and the project's collision-free prefix for workspace-owned Installer, Bootstrap, manifest, payload, checksum, and third-party fallback objects. GitHub may hold source, tags, release notes, and automation, but not permanent release binaries.

The client reads exact immutable object keys. It must not discover files through directory listing, filename guesses, GitHub, a package registry, a second mirror, or a runtime fallback URL. Upload immutable objects, read them back anonymously, and commit the one mutable Bootstrap last. A failed upload leaves the old Bootstrap usable.

## Distribution And Signing

Default workspace-owned delivery to `self-use` unless the user explicitly requests public production promotion or a channel requires publisher admission. Self-use application-owned binaries may use the workspace's unsigned provenance exception when final bytes are bound to exact source commit, object key, size, SHA-256, and doctor evidence. Third-party and platform-owned components retain their upstream provenance.

Explicit public production promotion requires the selected Windows Authenticode, notarization, store, or equivalent publisher provenance before the artifact is admitted. SHA-256 proves integrity, not publisher identity.

## Version And Publication

Use one canonical version owner per independently released component. Freeze version, bytes, object keys, size, digest, platform, architecture, and provenance before publication. Never overwrite an immutable same-version object or advance Bootstrap over an incomplete closure. A local installer or candidate is not a published release.

## Scope Guard

This policy does not authorize Deployment. Named target publication still requires the deployment workflow, artifact, target, admission evidence, authorization, rollback, and post-deployment checks.
