# Right-first-try preflight

Run before the first implementation commit of a feature. Each item exists
because skipping it has already cost a cycle in this repository.

1. **Read the lessons file.**
   `.claude/skills/fndr-v2-engineering/references/lessons.md` is the distilled
   memory of every mistake that cost time. Assume at least one applies to you.
2. **Navigate before you write.** Search the workspace for an existing
   implementation or near-miss; check the ADR-005 PORT list before writing
   from scratch. Parallel versions are how v1 got two rerankers.
3. **Name the tests before the code.** Write down the test names (including
   the negative and failure-path tests) the slice must ship with. If a test
   is hard to name, the seam is wrong.
4. **Enumerate the gates this change must pass.** fmt and clippy under
   `-D warnings` (edition 2024 rules included), workspace lints (egress,
   no-tauri), cargo-deny (a new dependency can fail bans or licenses
   transitively), AGENTS.md drift, bench baseline if ranking-adjacent, the
   bindings sync test if IPC-adjacent. Run the relevant ones locally before
   pushing, not in CI roulette.
5. **Verify third-party API shapes against the pinned source, not memory.**
   Crates in this stack (specta rc line, lancedb, rmcp, ureq) drift between
   versions; read the vendored source in `~/.cargo/registry` or fetch the
   crate before coding against a remembered API.
6. **Register every artifact where its runner expects it.** A migration file
   is not a migration until the runner's array includes it; a binding is not
   generated until the builder collects it; a CI step is not a gate until the
   workflow names it. After creating any registered-by-list artifact, grep
   for the list and confirm membership.
7. **State the typed failure path.** What does the user or caller see when
   this feature's dependency is missing or its operation fails? "Nothing"
   is the v1 answer and it is banned (invariant 4).
8. **Check the docs blast radius.** Which of PRD, roadmap, ADRs, ARCHITECTURE
   move together with this change? List them in the plan so the PR carries
   them, not a follow-up.
9. **Estimate the CI cost.** A heavy new dependency changes every future CI
   run (lance added ~17 minutes uncached). Know before you add.
10. **Confirm the branch and repo context.** Work happens in FNDR-2.0 on a
    feature branch; the guard hook resolves branches from the session cwd,
    so open sessions inside the repo you are changing.
