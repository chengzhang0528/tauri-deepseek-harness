# Client Verification Matrix

Scale verification to the changed boundary. This matrix distinguishes Development evidence from separately requested SystemTest and Deployment evidence.

| Changed boundary | Minimum Development evidence | Independent evidence only when requested |
|---|---|---|
| Contract, owner, or routing | source/formal-owner review and one positive/negative scope example | governance or system-wide audit |
| Installer or package graph | clean-output build, package inspection, version/architecture/size/digest checks | clean-machine installation, repair, upgrade, uninstall |
| Managed runtime | final-tree doctor, ABI/native-module load, readiness/health, bounded process cleanup | blank-machine bootstrap, source outage, recovery campaign |
| Existing Web UI shell | installed client loads the exact external product surface and adds no duplicate UI/IPC | full browser/desktop regression of the owned Web product |
| Update and activation | staging preserves current, confirmation/drain boundary, activation health, selected recovery path | repeated upgrade matrix, long-run checks, candidate acceptance |
| Publication | frozen candidate and local read-back verifier | public OSS admission, deployment, rollback |

## Positive And Nearby Negative Cases

Every changed boundary needs one positive case and one nearby negative case:

- A valid candidate reaches ready; a truncated, mismatched, or unhealthy candidate remains unactivated.
- An owned UI loads; a shell-owned fallback page or duplicate interaction is absent.
- A valid update stages and waits for the declared drain; active work is not forcibly interrupted.
- A final installed executable starts; a worktree/debug-only result is not accepted as installed evidence.
- A permitted source is read exactly; an outage produces a diagnosable failure without an undeclared fallback origin.
- Enabled providers produce normalized candidates; a higher but incomplete, incompatible, or unhealthy candidate is not selected, while a fixed source/version setting remains authoritative.

## Managed Runtime Checks

When the client owns a runtime, run checks against the final staged or archived tree. Do not rely only on dependency caches, a root-level package marker, readiness log timing, or a generic archive extractor. Verify the runtime's actual entrypoints, native modules, generated files, process tree, and post-staging paths.

## Reporting

Record the exact candidate or installed artifact, environment, versions, component source/outcome, current/staged state, health result, recovery mode/result, and remaining user action. Do not call these results a public release or deployment unless that task was explicitly established.
