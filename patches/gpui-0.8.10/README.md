# gpui submodule patches for ibisOffice 0.8.10

Upstream `ibisjp/ibis-office` (`c10a7d67`) updates the `forks/gpui-*` submodules. Because the CI/agent GitHub token is scoped to `sorane-dev/docs` only, the forked submodules under `masanork/gpui-*` could not be pushed directly from the VM.

These patches capture the exact diff between each upstream 0.8.10 submodule commit and the merged fork state:

| Submodule | Upstream 0.8.10 commit | Patch file |
| --- | --- | --- |
| `forks/gpui-base` | `8b62758` | *(fast-forward, no diff)* |
| `forks/gpui-pre` | `63dd0ee` | `gpui-pre.patch` |
| `forks/gpui-pre-macos` | `48e1e34` | `gpui-pre-macos.patch` |
| `forks/gpui-pre-web` | `6dbc49f` | `gpui-pre-web.patch` |
| `forks/gpui-pre-wgpu` | `2d25714` | `gpui-pre-wgpu.patch` |

## Reconstructing and pushing from a local environment

Run `./patches/gpui-0.8.10/apply.sh` from the repository root (or inspect the `.patch` files directly):

1. Fetches `https://github.com/ibisjp/<repo>.git` for each of the 5 submodules.
2. Fast-forwards `forks/gpui-base` to `8b62758`.
3. Creates a merge/patched branch on top of each upstream 0.8.10 commit for the remaining 4 submodules.
4. Review and push each submodule branch to `origin` (`masanork/gpui-*`), then update the submodule pointers in `sorane-dev/docs` if needed.
