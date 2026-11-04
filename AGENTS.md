# Repository Agent Instructions

## 1. Principles
- Prioritize correctness, speed, maintainability, and low resource usage.
- Work autonomously; avoid unnecessary confirmations and repeated operations.
- Inspect only relevant files; reuse known context and results.
- Prefer minimal, targeted changes over broad rewrites.
- Never claim unverified results.

## 2. Repository & Documentation
- Inspect relevant repository structure, code, tests, and documentation before editing.
- The GitHub repository `haotemans/youning` is the shared source of truth; use the current branch and remote commits rather than machine-specific paths.
- Maintain a clean, modular, consistent directory hierarchy.
- Keep source code, tests, documentation, and directory indexes synchronized.
- Update affected README, module docs, architecture docs, and file references.
- Maintain `module → feature → source → tests` ownership in `docs/features/catalog.json`.
- Keep one canonical feature document under `docs/features/<module>/`; plugin documents belong in `docs/features/extensions/plugins/`. Directory `README.md` files are indexes, not duplicate feature descriptions.
- Update the affected canonical feature document in the same PR as code, API, configuration, UI, or test changes; update the catalog when source or test paths change.
- Run `python scripts/check_feature_docs.py` before committing; use `--base origin/main` to check PR synchronization after committing. Link to detailed rules in [docs/governance.md](docs/governance.md).
- Distinguish implemented behavior, planned work, mock validation, and real-service verification; never use a design document or a green build as proof of live behavior.
- Place new files in their proper modules; avoid scattered files and duplicate implementations.
- Remove verified dead code, obsolete files, unused dependencies, and redundant documentation.
- Update imports, paths, and manifests when restructuring modules.
- Keep generated files, caches, binaries, and temporary outputs out of Git.
- Preserve useful build caches, user data, and unrelated changes.
- Avoid unnecessary restructuring or documentation duplication.

## 3. Implementation & Validation
- Follow existing architecture, conventions, and interfaces.
- Implement complete changes with appropriate tests and documentation.
- Run targeted tests, lint, and type checks before expensive full builds.
- For documentation-only changes, validate documentation, mappings, and links; do not manually trigger optional application builds or packaging. Existing required CI checks must still pass.
- Reuse dependencies, virtual environments, and incremental build caches.
- Windows: Build and package locally; use `npm.cmd` when necessary.
- Linux: Build and package via GitHub Actions, triggered using `gh`.
- Never use Windows GitHub Actions runners for routine builds.
- Trigger expensive packaging only when required.
- Do not claim platform-specific validation when the platform is unavailable.

## 4. Git & CI
- Check `git status` and relevant `git diff` before committing.
- Stage only related changes; never commit secrets.
- Commit completed work automatically with meaningful messages.
- Batch related changes and minimize remote pushes.
- Inspect `.github/workflows/` once; refresh only when changed.
- Push the completed task branch and identify CI for its exact commit; do not reuse an earlier commit's green result as evidence for a new head.
- Avoid redundant workflows, excessive polling, and full-log downloads.
- Monitor required CI using `gh run watch <run-id> --compact --interval 60 --exit-status`.
- Read failed logs only; fix root causes before retrying.
- GitHub Actions runners execute remote builds; `gh` only manages them.
- Report `Not Run` when no relevant CI was triggered.

## 5. Pull Requests & Merging
- Reuse existing PRs or create one when appropriate.
- Merge automatically when explicitly authorized and required checks pass.
- Prefer the repository's established merge strategy.
- Never force-push, bypass protection, or merge failing required checks.
- Verify remote commits, CI results, and final merge state.

## 6. Efficiency & Reporting
- Minimize token usage, network traffic, tool calls, and build time.
- Avoid repeated file reads, dependency installation, and unchanged tests.
- Prefer targeted searches, diffs, logs, and incremental verification.
- Clean disposable artifacts only when safe and necessary.
- Stop repeated attempts when a concrete external blocker is identified.
- Report concisely: changes, commit, push, CI, merge, and blockers.

## 7. Server Deployment
- Deploy only when the user requests a server update; code/CI-only work does not authorize deployment. Respect an explicit request to defer deployment.
- By default, build/update/restart only the `yuantuan` service. Keep the existing NapCat container running; do not recreate, redeploy, or restart NapCat without a separate explicit request.
- After preparing the image, update only the application with `docker compose -f deploy/compose.yml up -d --no-deps --no-build yuantuan`. Do not run untargeted Compose `up`, `down`, or `restart` during an application update.
- Do not apply NapCat Compose changes as part of a normal application deployment; preserve its container and named volumes to retain QQ login state.
- Preserve databases, runtime configuration, provider credentials, plugin settings, enablement state, and unrelated runtime data. Back up before switching versions and retain a usable rollback image/artifact.
- Use the GitHub-built Linux artifact for the exact validated commit; verify its checksum and the running revision. Never overwrite server configuration with a repository template.
- Check application health and the existing QQ connection after deployment; distinguish a healthy process from successful model calls or confirmed media delivery.
- If SSH or another external dependency is blocked, preserve prepared artifacts and report the blocker; do not claim deployment succeeded.

## Execution Order
Inspect → Implement → Update Docs & Structure → Validate → Commit → Push → Check CI → Merge (if authorized).

**Keep code clean, documentation accurate, project structure consistent, and operations efficient.**
