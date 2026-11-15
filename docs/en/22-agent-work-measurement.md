# 22. Agent Work Measurement

[한국어](../22-agent-work-measurement.md) | **English**

Apply this process to work starting on 2026-11-15. Link available historical evidence without inventing retrospective scores. The unit is a task with a verifiable outcome, identified as `WORK-YYYYMMDD-NN`, rather than a chat message. Dates use Asia/Seoul; sequence numbers must be unique within each date.

## Records and timing

- Start: copy the [request template](../../inbox/TEMPLATE.md) and register it in the [current work index](../work-index.md). Record the user request, supplied context, baseline commit and working tree, scope, relevant REQ/TASK/ADR/TEST references, and acceptance criteria. Explain when references are not applicable.
- During work: append requirement changes, design intent, failed approaches and corrections, human decisions, and actual delegation in chronological order. Preserve original criteria and record the reason and decision owner for additions or exclusions.
- Finish: use the [result template](../../outbox/TEMPLATE.md) to link each criterion to commands, environment, results, and evidence. Distinguish changed files, commits/PRs, and integration targets/results. Identify local-only changes as uncommitted and not integrated.
- Follow-up: append later defects to the original result with discovery date, impact, reason missed, correction task, and revalidation. No discovered defects is not a guarantee of zero defects.

Use the same WORK ID in request and result filenames. The index summarizes locations and status without duplicating details. Status is IN PROGRESS / BLOCKED / DONE; DONE means the task's acceptance criteria are met. Integration or deployment requires evidence when in scope. Record exclusions and deferrals as changes. Track unresolved follow-up defects separately in the index.

## Four criteria and metrics

| Criterion | Required evidence | Measurement |
|---|---|---|
| Task, context, and verification handoff | Original request, references, scope, acceptance criteria, actual instructions when delegated | Tasks with criteria recorded before execution / tasks started in period; rework incidents caused by missing context |
| Requirements and design continuity | REQ/ADR→acceptance criterion→changed files→verification; change reasons and owners | Criteria with PASS evidence / currently applicable criteria; criteria changed without justification |
| Verification, integration, and missed defects | Execution results, version/environment, integration evidence, later defects | Integration failures; defects discovered after completion and their impact |
| Human judgment and failure correction | Reason for escalation, options, decision owner/result, failure cause, correction and revalidation | Corrected approaches revalidated / corrected failed approaches; recurrence of the same cause |

Report the period, included WORK IDs, numerator/denominator, and observation cutoff. Use N/A for a zero denominator. Disclose original criterion counts and additions/exclusions to prevent improving ratios by removing criteria. Distinguish duplicate reports of the same defect and describe actual impact, such as data loss or blocked functionality. Fewer questions or more tests alone do not imply better performance.

Verification results are PASS / FAIL / SKIP / NOT RUN. A successful process that skipped DB tests is not a DB verification PASS. Preserve reproducible commands, sanitized result summaries, and evidence locations instead of full sensitive logs. External CI evidence includes the run URL and target commit. An ignored local log that may disappear must not be the only evidence.

Optionally score traceability per criterion: 0 (no record), 1 (description only), 2 (evidence), 3 (linked request, outcome, and follow-up). This is not a software quality score. Distinguish missing evidence from not applicable.

## Maintenance

Existing requirements, ADRs, and test documents remain authoritative; work records link to them. Preserve the historical [development status](20-development-status.md). Track current tasks in the separate index.

Update numbered design documents, their language counterparts, and generated outputs together. The work index and request/result records are shared Korean operational records; per-task translations are not required. This English document links to the same records. Record reviewable decision reasons and execution evidence, not secrets, personal data, or private internal reasoning.
