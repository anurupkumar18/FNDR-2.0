# Feature plan template

Keep the whole plan under a page. Delete sections that honestly do not apply
rather than padding them.

```
## <feature name>

Problem: <one sentence: who hurts, when, and how this fixes it>
PRD tie: <goal G1..G5 or pain-point row; or "none" and stop>
User: <builder / agent / evaluator>
Smallest valuable version: <the cut line>
Demo relevance: <none | which beat, product capability only>

ADR touchpoints: <ADRs read; conflicts and how resolved; amendments needed>
Existing code/tickets reused: <what was found in the navigate-before-write search>

Slices (each = behavior + tests + docs, one reviewable unit):
1. <slice, with its named tests>
2. ...

Bench/eval impact: <none | which metric could move; baseline plan>
Typed failure path: <what shows when dependencies are missing or calls fail>
Docs moving together: <PRD / roadmap / ADR / ARCHITECTURE edits in the same PR>
Gates: <the preflight item-4 list relevant to this change>

Tickets: <new or re-scoped ticket lines in roadmap format>
```
