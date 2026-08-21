# Managed Runtime Acceptance

Load this reference when a client ships or prepares a private language runtime, CLI closure, native modules, generated bridge, or long-lived child process.

## Closure

Freeze the runtime version, platform, architecture, source identity, exact files, and digest before packaging. Build the closure once from a clean output directory. The user's machine must not silently fall back to a system runtime, an unpinned registry install, or an undeclared binary origin.

Run final doctor checks against the exact staged or archived tree:

- runtime and application version commands;
- all declared native modules on the target ABI;
- required helper tools and generated files;
- licenses/notices and expected entrypoints;
- readiness or health behavior using the real launch command;
- bounded stdout/stderr, timeout, cancellation, and process-tree cleanup.

Do not infer health from a readiness log alone. A readiness event may race the first usable HTTP/UI response; retry the contract-defined health request within a bounded window and reject an identity mismatch.

## Staging And Activation

Use fresh staging and the production unpacker. Re-materialize generated configuration or bridge files after staging if they contain absolute paths or staging-dependent locations. Validate the final paths, not only the build-time paths.

Keep `current` unchanged until every required component and doctor check passes. A failed candidate may clean only its own staging and temporary files. Activation must preserve the selected recovery target or forward-repair state and must not delete user data.

## Runtime Process

Give each owned root process one explicit lifecycle owner. Use graceful shutdown and drain when the managed application exposes it; reserve process-tree force termination for the documented emergency boundary. Do not use TCP reachability, a fixed port, or an unknown loopback service as the runtime identity proof.

## Product Boundary

The runtime manager may own executable preparation, process lifecycle, and diagnostics. It must not take ownership of the embedded product's pages, business state, credentials, task data, or model interaction unless the product contract explicitly assigns those responsibilities.
