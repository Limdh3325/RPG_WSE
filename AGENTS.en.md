# Project Working Rules

[한국어](AGENTS.md) | **English**

- Respond to the user in Korean. Explain results and the reasons for decisions clearly.
- Before starting work, inspect relevant code and documents and follow the user's goals, scope, and constraints. Do not record personal information or secrets.
- Follow the [measurement rules](docs/en/22-agent-work-measurement.md) and the [current work index](docs/work-index.md).
- At task start, copy `inbox/TEMPLATE.md` to create a request record with a `WORK-YYYYMMDD-NN` ID and register it in the index. Record the baseline commit and working tree state, relevant requirements and design, and acceptance criteria. Keep small tasks concise without omitting required fields.
- Proceed within the scope already authorized by the user. When human judgment is needed, record the options, impacts, and decision outcome. If no work was delegated, state that explicitly.
- When requirements or approaches change, preserve the original criteria and record the reason, timing, and decision owner. Follow the existing ADR process for significant design changes.
- On completion, use `outbox/TEMPLATE.md` to create a result record with the same ID. Link evidence for each acceptance criterion, failures and corrections, unverified items, and actual integration status. Append later defects to the original work record as well.
- Distinguish verification results as PASS / FAIL / SKIP / NOT RUN. Do not report unexecuted checks, skipped DB tests, or local success as integration or production success. If there is no commit or PR, state that explicitly.
- Update source documents and their Korean/English counterparts together. Do not edit the integrated design documents or manifest directly; generate and check them with `node scripts/sync-design-docs.mjs` and its `--check` option.
- Base work records on actual observations and evidence. Do not fill historical records with assumptions or overwrite historical status tables with claims of current completion.
