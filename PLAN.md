# Dermixen build plan

The work of building Dermixen is divided into chunks, each done by one role and each shown correct by something that runs. `DESIGN.md` is the specification. This plan is the working method: how work is verified, the roles, the packet a role receives, and the harness settings the roles rely on. The role definitions live in `.claude/agents/`, and the harness limits live in `.claude/settings.json`.

## How work is verified

Work is verified by ear, by scoreboard, and by running things, and code review reads the diff as well. Every claim of correctness has something that runs behind it. Four rules follow from that, and every piece of work obeys them.

1. **Contracts before code.** Public types, trait signatures, and the project file schema are written and compiled as stubs before any implementation starts. Coders fill in bodies against fixed interfaces.
2. **Tests before implementation, by a different agent.** The acceptance tests for a chunk are written by the lead and committed before the implementation exists, marked ignored with the chunk's name as the reason so that continuous integration stays green. `cargo test -- --include-ignored` runs them and shows them failing. The coder's job is to make them pass without editing them, and the lead removes the ignore markers when the chunk merges. An ignored test is therefore a chunk that is not done yet. An agent that writes both the test and the code will, with the best intentions, write a test that passes. Separating the two is what makes a green test evidence in its own right, and code review then reads the code behind it.
3. **Every chunk has a runnable done-criterion.** `cargo test -p dermixen-core`, `dermixen render two.dmx out.wav`, `scoreboard report`. A chunk is done when that command runs and succeeds. The repository is the status board. This plan does not track per-chunk status.
4. **Mechanical gates the compiler enforces.** Every crate except the FFI wrappers has `#![forbid(unsafe_code)]`. Positions, durations, tempos, and beat counts are distinct types, so mixing them up is a compile error, not a bug found by ear. Continuous integration runs the formatting check, the lints, the tests, and a `cargo deny check` (licenses, security advisories, dependency bans, and dependency sources) on Linux and macOS for every push to `main` and every pull request, and `scripts/check.sh` runs the formatting check, the lints, and the tests locally.

## Roles

Each role is a subagent definition in `.claude/agents/`. A work packet names the role. The definition fixes the model, the effort level, the tools, and the standing instructions.

| Role | Model and effort | Work |
| --- | --- | --- |
| `lead` | Fable, high | Owns `DESIGN.md` and this plan. Writes the contracts, writes or assigns acceptance tests, integrates finished chunks, reviews the risky ones. Runs long-lived and coordinates the other roles asynchronously. |
| `researcher` | Fable, xhigh | The unsolved problems: bespoke downbeat, phrase, and anchor-placement analysis measured on the scoreboard, and decoding where MixMeister project files keep anchor positions. |
| `coder-hard` | Opus, xhigh | Chunks with subtle math or build-system risk: the tempo-curve mapping, the FFI shims, the render graph, real-time preview. |
| `coder` | Opus, high | Standard chunks against a fixed interface. Stays alive across the chunks of one crate so context is not rebuilt each time. |
| `coder-bounded` | Sonnet, high | Small, fully specified chunks: a WAV writer, envelope interpolation, serialization, a lookup table. |
| `reviewer` | Opus, low | Independent review of a returned chunk. Reports everything it finds. The lead filters. Cannot edit files. |
| `mechanic` | Sonnet, low | Continuous-integration configuration, fixture generators, formatting, prose sweeps. |

Each role names its model by the alias in its definition (`fable`, `opus`, or `sonnet`), so a role runs on the current model of that tier. Fable is the right tier for work that is long, ambiguous, or unsolved, and for coordination. Opus is the right tier for anything with a specification and tests. Nothing in this plan can only be done by Fable: if it is unavailable, an Opus lead at xhigh effort takes over coordination and the research problems proceed more slowly.

## The work packet

A packet is the complete specification a coder receives. It holds:

- The crate and module path, and the stub signatures already compiled in the repository.
- The failing tests the chunk must make pass, and the rule that the tests are not to be edited.
- The files the chunk may touch, and the files it must not.
- The runnable done-criterion.
- The reason: who the work is for and why the tests are how it is trusted. Models perform better when they know the intent, and this project's intent is unusual.
- A required closing report: what was done, in plain language, opening with the outcome, and with every claim backed by a test run or command output from the session.

Packets state the goal, the constraints, and the tests. They do not enumerate implementation steps. Current models do better with the goal than with a recipe.

Two instructions differ by model. Packets for Opus roles contain no instruction to verify or double-check. Opus verifies on its own, and an explicit instruction makes it over-verify. Packets for Fable roles that run for a long time do the opposite: they ask for a checking method run at an interval, using fresh-context verifier subagents, because those outperform self-critique on that model. Both rules describe the current model of each tier and are re-checked against Anthropic's migration notes whenever an alias moves to a new release.

Packets ask for a report of what was done and why the design holds. They do not ask an agent to explain its reasoning, which is phrasing that can trigger a refusal on Fable.

## Harness settings

- `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH` is 1, so a coder cannot spawn a team of its own.
- `CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS` is 6.
- Coders run in their own git worktree so parallel chunks do not collide.
- Coders that own a crate stay alive across that crate's chunks and are continued by message rather than respawned.

## Known risks

- **Golden references are self-referential.** The MixMeister project fixtures cannot serve as reference audio while their anchor positions are undecoded, so the golden renders freeze what Dermixen produces and detect drift from there. Listening to the render is the real gate.
- **Foreign-function builds across two operating systems** are the likeliest place for a coder to stall. Those chunks go to `coder-hard`, and every such dependency is feature-gated.
- **Interface churn** is the main cost of running many coders on one crate. Concurrency stays low while a crate's contracts are unproven.
- **Render throughput.** A render of an 88-minute mix takes 39 minutes of wall time and six minutes of processor time, so the render path spends most of its time waiting on something not identified. The wait does not affect what is rendered.
- **Fable availability.** The role table above degrades to Opus without losing any capability the plan depends on.
