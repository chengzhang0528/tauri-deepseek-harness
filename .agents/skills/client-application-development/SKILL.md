---
name: client-application-development
description: Guide fast, bounded development of desktop, launcher, updater, native, mobile, and web clients. Use for Chinese requests about 技术选型、开发客户端、安装包、更新、版本、发布、签名、回滚、OSS or similar client delivery work. Keep reusable method rules here; load platform, distribution, and managed-runtime variants from references only when the request needs them.
---

# Client Application Development

## Purpose

Use this skill to turn a client request into the smallest verifiable implementation. The main skill owns the method: scope, ownership, delivery choice, lifecycle, evidence, and stop boundaries. Product facts remain in the project's ProductContract/CurrentDesign/Runbook, and platform or distribution details remain in references.

Do not infer a SystemTest or Deployment task from build success, CI, a tag, a candidate, an installer, Git push, or the breadth of validation. A feature, fix, or governance change plus its focused checks remains Development unless the user separately requests an independent test or deployment result.

## Start With The Fast Path

For the first pass, load [client-fast-path.md](references/client-fast-path.md) and complete its one-page worksheet before reading specialized references.

1. Read the root and project entrypoints, then the smallest formal owner for the requested client facts.
2. Classify this request as Development, SystemTest, or Deployment from the user's goal.
3. Fill the minimum client contract: product role, UI owner, platform/architecture, install source, runtime owner, update owner, distribution mode, recovery mode, and completion rule.
4. If an existing product owns the UI, keep that UI as the only visual owner; do not invent a shell page, fallback page, or duplicate interaction.
5. Choose thin/full/hybrid delivery only from measured payload size, offline requirements, prerequisites, and component ownership.
6. Build one real candidate, run the final component/launcher doctor, and verify the installed executable or target artifact rather than only the worktree.
7. Report Development evidence separately from any later SystemTest, Deployment, and Git result.

If a decision changes the outcome, ownership, persisted/public contract, compatibility promise, or external target, stop and expose two or three mutually exclusive choices. Do not stop for discoverable implementation details.

## Minimum Contract

Resolve each field from the authority shown below. Keep working assumptions visible and reversible; never promote them to requirements merely because a reference or earlier plan used them.

| Field | Required question | Preferred source |
|---|---|---|
| Product role | Is this a product, shell, launcher, installer, updater, or managed capability? | User request + ProductContract |
| Visual owner | Which product owns HTML, CSS, routes, and user interaction? | ProductContract + source |
| Platform | Which OS/device/browser and architecture are supported now? | ProductContract |
| Install source | Which installer, package manager, store, or official framework path owns install and repair? | ProductContract + platform reference |
| Runtime owner | Which component owns executable preparation, health, and process cleanup? | CurrentDesign + source |
| Update owner | Which official mechanism updates each independently versioned component? | CurrentDesign + technology reference |
| Distribution | Is this self-use, internal, controlled, or explicit production promotion? | User request + ProductContract/Runbook or distribution reference |
| Recovery | Is post-activation recovery automatic rollback or forward repair? | ProductContract/CurrentDesign |
| Completion | What observable result completes this task? | User request |

When sources conflict, repair the single fact owner before expanding scope. Do not copy project facts into this skill.

## Ownership And Lifecycle

Use one owner per responsibility:

| Responsibility | Owner rule |
|---|---|
| Product behavior and UI | The product that owns the user-facing behavior; a shell must not duplicate it |
| Installer/platform integration | The platform installer, store, or package manager |
| Runtime preparation and process tree | The launcher or native host |
| Update intent and user confirmation | The existing client/manager surface or native shell explicitly named by the contract |
| Release publication | The approved release workflow and immutable source |
| Product data migration | The owning application, only for enumerated mappings |
| Workspace governance | AGENTS, WorkflowContract, StructureContract, TASK_CONTROL, or this skill at their actual owner |

The reusable lifecycle is:

`discover -> contract -> choose delivery -> build -> verify -> stage -> activate -> recover -> closeout`

Keep binaries, configuration, credentials, user data, and migrations in separate ownership boundaries. Prepare candidates in temporary state, verify size/digest/provenance and declared health, preserve the current runnable release until activation is safe, and use the contract-selected recovery mode after activation. Never add a fallback origin, compatibility bridge, service, scheduled task, or custom transport-security policy without an explicit owner and requirement.

## Delivery And Update Decisions

Choose the existing framework/package-manager/platform path before introducing a new shell, installer, updater, runtime, or release stream. Load only the reference that matches the decision:

- [client-contract-template.md](references/client-contract-template.md) for fields, provenance, and owner/source checks.
- [thin-installer.md](references/thin-installer.md) for thin/full/hybrid delivery, component reuse, staging, and activation.
- [technology-and-update-channels.md](references/technology-and-update-channels.md) for stack selection and official update owners.
- [lifecycle.md](references/lifecycle.md) for state ownership, drain, activation, and recovery.
- [managed-runtime-acceptance.md](references/managed-runtime-acceptance.md) when the client owns a private runtime, native modules, a long-lived child process, or a generated bridge.
- [platform-windows.md](references/platform-windows.md) for Windows installer, hidden process, WebView2, upgrade, and uninstall behavior.
- [release-and-versioning.md](references/release-and-versioning.md) and [distribution-and-signing.md](references/distribution-and-signing.md) only when release, signing, or immutable publication is in scope.

Do not read every reference by default. The fast path selects the smallest set.

## Execution Boundaries

### Development

Implement the requested client behavior and run affected source/type checks, focused tests, final candidate doctor, and a proportionate installed-artifact smoke check. A local installer is a Development candidate, not a public release.

### SystemTest

Run only when the user explicitly asks for an independent integration, system, full-suite, candidate-acceptance, or regression result. Fixes found there require a separate Development authorization.

### Deployment

Run only when the user explicitly names a release artifact and target. Require the deployment workflow's admission evidence, authorization, rollback, and post-deployment checks. OSS publication and production signing are not implied by Development completion.

## Verification Floor

Use [client-verification-matrix.md](references/client-verification-matrix.md) to scale evidence to the changed boundary. At minimum, verify contract/owners, affected source/types/tests, final-tree doctor, startup/readiness, process cleanup, user-state preservation, and the selected update, drain, activation, and recovery behavior when changed.

Black-box public-asset acceptance, clean-machine testing, OSS outage testing, and deployment admission belong to separately requested SystemTest or Deployment work.

## Stop Conditions

Stop before implementation when any of these cannot be resolved from the user, project owner, source, or the routed reference:

- the supported platform/architecture or installer source is unknown;
- product/UI ownership is ambiguous or the proposed change would duplicate another product's UI;
- runtime, update, or recovery ownership is missing;
- a public or persisted compatibility promise is implied but not owned;
- the requested action would change an external target without explicit Deployment authorization;
- final candidate bytes, installed behavior, or required health evidence cannot be observed;
- a required signature, credential, or platform prerequisite is missing for the selected production/channel contract.

Missing self-use publisher credentials do not block local Development when no production admission requires them; the selected distribution owner defines the exact exception.

## Report And Close

Report the task type, changed boundary, resolved owners, delivery shape, measured artifact/installed evidence, recovery result, unsupported cases, and Git state separately. Use WF-0004 for workspace-changing tasks. Do not create task state, candidates, or permanent documents merely to record this method run.
