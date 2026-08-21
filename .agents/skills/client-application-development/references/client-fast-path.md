# Client Fast Path

Use this reference for the first pass on a client request. It is a worksheet, not a replacement for the formal product owner or the platform-specific references.

## Ten-Minute Worksheet

### 1. Request boundary

Record one sentence for the observable result and one sentence for what is explicitly out of scope. Classify the request from the user's goal:

| User goal | Execution type |
|---|---|
| Build, change, fix, refactor, or govern client behavior | Development |
| Independently test an existing workspace or fixed candidate | SystemTest |
| Publish, release, migrate, switch, or roll back a named artifact/target | Deployment |

Do not infer the last two rows from a build, candidate, tag, CI job, or test breadth.

### 2. Minimum contract

Fill these fields before choosing a framework or installer:

| Field | Answer |
|---|---|
| Product role | product / shell / launcher / installer / updater / managed capability |
| Existing UI owner | this client / another product / no UI |
| Platform and architecture | exact supported target |
| Install source | installer, package manager, store, or official framework path |
| Runtime owner | installer, launcher, client, platform, or external product |
| Update owner | one owner per independently versioned component |
| Delivery shape | thin / full / hybrid, with measured reason |
| Recovery | automatic rollback / forward repair / external replacement path |
| Completion | observable result for this request |

If the UI owner is another product, the shell may own native lifecycle and diagnostics but must not add a web page, fallback page, duplicate route, or business interaction.

### 3. Choose the smallest path

| Decision | Use this evidence | Load next |
|---|---|---|
| Existing stack is sufficient | current source and official packager/update path | technology-and-update-channels.md |
| Payload is large or replaceable | measured installer, first-run, and installed sizes; offline requirement | thin-installer.md |
| Client owns a private runtime | final archive/tree, ABI, native modules, readiness, process cleanup | managed-runtime-acceptance.md |
| Windows packaging is in scope | installer scope, privileges, prerequisites, repair/upgrade/uninstall | platform-windows.md |
| Release or publication is in scope | version owner, immutable assets, source admission, signing | release-and-versioning.md + workspace-delivery-defaults.md |
| Only local Development is in scope | source/types/tests, final candidate, installed executable | client-verification-matrix.md |

Load one row's references first. Add another only when the decision reaches that boundary.

### 4. First candidate

Build once from a clean output directory. Verify the exact candidate that will be installed or archived:

- declared files and entrypoints exist;
- version, platform, architecture, size, digest, and provenance match the contract;
- final component/launcher doctor runs against the candidate tree;
- the installed executable starts, reaches its contract-defined health signal, and leaves no owned process behind after exit;
- failure before activation leaves the current release and user data unchanged.

### 5. Stop and report

Stop when an owner, platform, installer source, recovery mode, compatibility promise, or required external authorization is missing. Report Development, SystemTest, Deployment, and Git results separately. A local candidate is not a public release.
