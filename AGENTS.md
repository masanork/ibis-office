# masanork fork workflow

- `masanork` is this fork's integration branch and the base for new feature
  and fix branches. `main` tracks upstream; `origin` is `ibisjp/ibis-office`,
  and the `masanork` remote is `masanork/ibis-office`.
- Develop each independent change on its own branch, using an isolated
  worktree when the current checkout contains unrelated changes.
- Integrate completed, tested changes into `masanork`. Verify the final
  integrated head, push it, and then delete the completed feature branches
  locally and remotely. Preserve active branches and dirty worktrees.
  Check ancestry or document patch equivalence before deleting a branch.
- Use the repository-local Git identity `masanork` and
  `2642715+masanork@users.noreply.github.com` for both author and committer.
  Check author metadata when cherry-picking older commits.
- Keep local JTD/PDF samples and generated screenshots out of Git and public
  assets. Use the existing synthetic fixtures for distributable regressions.
- The deployed fork is <https://ibis.sorane.dev>. Preserve its Custom Domain
  and the Worker target in `wrangler.jsonc` when deploying.

See [the fork workflow](docs/fork-workflow.md) for commands and validation.
