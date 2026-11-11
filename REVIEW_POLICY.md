# Review policy

This policy applies to every pull request, whoever wrote it.

## 1. Classify every PR

**Trunk**: the change touches shared code that other parts depend on:

- The scanner and classification of template, local-override, and upstream-conflict states
- The write path: merge, backup, confirm, and undo
- Public CLI flags, config formats, and the crate's public API
- The Omarchy shell-plugin and Hyprland launcher integration
- CI, release, and build configuration, including `Cargo.toml` and `Cargo.lock`
- Anything with 5 or more dependents

**Leaf**: everything else, such as a single view polish, documentation, a self-contained script, test-only changes, or a capture of an existing view.

A PR that is partly trunk is trunk. When unsure, call it trunk. Label PRs `trunk` or `leaf`.

## 2. Proof

The PR body has a **Proof** section with real evidence: test output, a CI run link, a capture for visual changes, or a before/after for behavior changes. "Tested locally" without output is not proof.

Pull requests that add or change a visible lunatui widget or TUI view must include a README GIF or WebP captured from real program output, not a mock and not a screenshot of a different tool.

## 3. Independent peer review

- Every PR gets a review from someone other than the author.
- The reviewer names the exact tip commit they reviewed and answers **SHIP** or **CHANGES**. A new commit needs a new review at the new tip.
- A SHIP carries over a clean rebase or a label-only change when the PR's own diff is unchanged (`git range-diff` or `git patch-id --stable` shows no difference).
- Trunk PRs are also built and exercised by the reviewer, not just read. Every finding is fixed or explicitly waived in the PR thread.
- Docs-only and changelog-only PRs may merge on green checks with no review.

## 4. Required checks

All required status checks on `main` must pass before merging. No check is skipped, and code-scanning threads are resolved only when addressed or report-only.

## 5. Feature gating

New behavior in trunk code ships on by default. A Cargo feature or setting that is off by default is used only when the owner explicitly asks for one for that feature. A pure fix never needs a flag. Turning on a feature the owner asked to gate is its own trunk PR.

## 6. Tests

Every test must fail if the behavior it names broke. Reject tests that assert a value against itself, compute the expected value with the code under test, assert what a mock of the unit under test returns, only check that nothing panicked, compare against unreviewed regenerated snapshots, or still pass when the implementation is replaced with a stub.

## 7. Merging

- Squash only. No merge commits, no rebase merges, no admin overrides, and no force-pushes to `main`.
- Commits carry no private details: no personal names, paths, hostnames or addresses.
- Releases and tags need owner approval.

## Local verification

Every behavior-changing PR must include proof that the change was built and exercised in a clean local VM (a Tart macOS VM, or a clean VM of the relevant OS, including Omarchy/Linux for host-integration changes) and shown working: the commands run plus their output, or a screenshot.

- Reviewers must not give SHIP without that proof.
- Docs-only and CI-only PRs are exempt.
- Only an explicit owner override waives this requirement.
