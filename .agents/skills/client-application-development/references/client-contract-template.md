# Client Contract Template

Use this reference to capture the minimum facts needed to start implementation. Keep the answers in the project's formal owner when they become durable product or design facts; this template is not a second ProductContract.

## Contract

```text
Outcome:
In scope:
Out of scope:
Execution type: Development | SystemTest | Deployment

Product role:
Existing UI owner:
Supported platform/architecture:
Install source and repair owner:
Runtime/process owner:
Update owner per component:
Distribution mode: self-use | internal | controlled | production
Post-activation recovery:
User confirmation boundary:
Active-work drain rule:
Completion rule:
```

## Source And Provenance

For every material answer, classify it as one of:

- Confirmed requirement: explicit user instruction or bounded delegation.
- Verified fact: source, types, tests, measured behavior, or formal owner.
- Working assumption: reversible recommendation that still needs confirmation or evidence.

Silence, repetition, prior-plan inclusion, and copied wording do not turn an assumption into a requirement.

## Owner Matrix

| Fact or behavior | Sole owner | Do not copy into |
|---|---|---|
| User-visible product behavior | ProductContract and source | skill or installer reference |
| Capability architecture | CurrentDesign and source/tests | product contract or generic skill |
| Commands and recovery procedure | Runbook | skill or README copy |
| Workspace delivery defaults | workspace-delivery-defaults.md | ProductContract or main skill |
| Reusable client method | client-application-development/SKILL.md | project docs |
| Platform or delivery variant | the relevant reference | main skill |
| Authorized recoverable activity | TASK_CONTROL and its plan | ProductContract or skill |

When two sources disagree, stop expansion and repair the owner. Do not preserve both versions as caveats.

## Completion Check

The contract is ready for implementation only when the requested user result, change boundary, UI owner, platform, installer/runtime/update owners, recovery behavior, and observable completion rule are all resolved. Later publication or independent system testing remains a separate task.
