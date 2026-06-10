---
tags:
  - release
  - moc
---

# Release Map

The first release should be intentionally narrow: one Linux/NAS queue, one Windows worker, and one macOS companion for a trusted user.

```mermaid
flowchart TD
  A["Limited release candidate"] --> B{"Safety gate"}
  B -->|"pass"| C{"Install gate"}
  B -->|"fail"| B1["Fix verification semantics<br/>and original-delete guardrails"]
  C -->|"pass"| D{"Ops gate"}
  C -->|"fail"| C1["Package worker + companion<br/>document queue deploy"]
  D -->|"pass"| E{"Smoke-test gate"}
  D -->|"fail"| D1["Health, logs, backup/restore,<br/>known recovery steps"]
  E -->|"pass"| F["Limited trusted release"]
  E -->|"fail"| E1["Add fixture-based end-to-end test"]
```

## Release Notes

- [[05-Product/Limited Release Checklist|Limited Release Checklist]]
- [[03-Platforms/Platform Matrix|Platform Matrix]]
- [[04-Operations/Runbook|Runbook]]
- [[06-Risks/Open Questions|Open Questions]]

## Suggested Milestones

```mermaid
gantt
  title Limited Release Path
  dateFormat  YYYY-MM-DD
  axisFormat  %b %d
  section Safety
  Server verification semantics      :crit, a1, 2026-06-09, 3d
  Delete-original guardrails         :crit, a2, after a1, 1d
  section Packaging
  Worker install/update notes        :b1, 2026-06-10, 2d
  macOS companion packaging          :b2, after b1, 2d
  section Operations
  Smoke fixture and recovery runbook :c1, 2026-06-12, 2d
  Release checklist dry run          :milestone, c2, after c1, 0d
```

