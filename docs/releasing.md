# Releasing

This fork primarily publishes Windows x64 releases to `IKurokami/strata`.
Windows packages use a separate signing key and manifest from upstream Linux
packages. The tag grammar below is shared by both updaters.

## Windows publication

1. Merge the release PR, including the stable package version in `Cargo.toml`
   and `Cargo.lock`. Record the exact source commit and tree. For an RC, keep
   the package version at its core version and inject the full RC tag.
2. Use the native UCRT64 toolchain described in
   [Windows development](windows-development.md). Set `STRATA_RELEASE_TAG`,
   `STRATA_BUILD_KIND` (`stable` or `rc`) and `STRATA_BUILD_COMMIT`, then build
   both `strata` and `strata-shell` in release mode. Preserve build caches.
3. Run `scripts/windows/package-release.ps1` with a fresh destination and the
   verified private runtime. It stages dependencies, runs portable smoke checks
   and writes the file manifest. Run `scripts/windows/package-installer.ps1`
   against that complete bundle to create the per-user installer.
4. Archive the complete bundle as
   `strata-VERSION-x86_64-pc-windows-gnu.zip`. Sign its updater manifest with
   `scripts/sign_update_manifest.py` using a key trusted by
   `data/windows-update-keys.json`. Keep private keys outside the repository.
   Publish the manifest, signature envelope, ZIP, installer and `SHA256SUMS`.
5. Create a draft GitHub release targeting the recorded merged commit. Verify
   asset sizes, checksums, runtime provenance and signature before publishing.
   A stable release is an ordinary release marked Latest; an RC is a prerelease.
6. Verify the public release feed, tag commit, signed manifest and downloaded
   artifacts without authentication. Record installed upgrade, restart and
   rollback checks separately; package verification alone does not establish
   that those installed paths were exercised.

The retained **Release** Actions workflow below builds Linux artifacts. It does
not build or publish this fork's Windows installer. Do not invoke it as a
substitute for Windows packaging or claim its attestations cover local builds.

## Upstream Linux signing setup

Before publishing, configure the `release-signing` environment, its default-branch
restriction and required reviewer, and its `UPDATE_SIGNING_KEY` secret as described
in [Signed in-app updates](signed-updates.md). The public key must be embedded in
`data/update-keys.json`. Signing is mandatory: a missing key stops publication
before the tag is pushed. The runbook also covers dual-signature key rotation,
secure key backups, and the transition from existing unsigned releases.

## Tag grammar

Exactly these forms are valid release tags. The self-updater rejects anything else outright, since this is the only contract it parses:

| Form | Example | Meaning |
| --- | --- | --- |
| `v?MAJOR.MINOR.PATCH` | `v0.5.0` | A final, stable release. |
| `v?MAJOR.MINOR.PATCH-alpha.N` | `v0.5.0-alpha.1` | An alpha build. |
| `v?MAJOR.MINOR.PATCH-beta.N` | `v0.5.0-beta.1` | A beta build. |
| `v?MAJOR.MINOR.PATCH-rc.N` | `v0.5.0-rc.1` | A release candidate. |
| `v?MAJOR.MINOR.PATCH-nightly.YYYYMMDD[.N]` | `v0.5.0-nightly.20260901` | A nightly build dated `YYYYMMDD`, with an optional same-day disambiguator. |

`N` is always a positive integer and is compared numerically. The leading `v` is optional in the parser but always present in published tags. At an equal core version, builds order as `nightly < alpha < beta < rc < stable`.

The in-app **Preview** channel receives alpha, beta, RC, and stable releases. The **Nightly** channel additionally receives nightly builds. All prerelease kinds, including nightlies, are published manually through the Release workflow.

## Publishing a stable release

Run the **Release** workflow from GitHub's Actions tab on the default branch, choose a `bump` (`patch`, `minor`, or `major`), and leave `mode` at its default, `stable`. Once both Linux targets build:

- the `prepare` job refuses to proceed if a release candidate tag exists for the target core version whose commit is not yet reachable from the release source -- promote or discard that RC first, so a stable release can never silently supersede an untested one;
- after maintainer approval, the isolated `sign` job signs the manifest using tools from the workflow revision, binding both archive digests to the tag and original build source commit;
- after the separate release approval, the `release` job commits the new version into `Cargo.toml` and `Cargo.lock`, tags the commit `vX.Y.Z`, and pushes both to the default branch; and
- it publishes x86-64 and ARM64 archives, matching debug-symbol files, checksums, the signed manifest, and build-provenance attestations as an ordinary (non-prerelease) GitHub release -- the endpoint a Stable install polls.

## Cutting and promoting prereleases

Run the same workflow manually with `mode` set to `alpha`, `beta`, `rc`, or `nightly`. For a prerelease, the optional `source` input accepts a pull-request number, branch, tag, or commit; leaving it blank uses the current default-branch tip. The workflow:

- resolves and records the exact source commit before building;
- computes the next core version from `bump`;
- gives alpha, beta, and RC builds the next numeric ordinal for their stage;
- gives nightlies a UTC-dated suffix, adding `.N` for repeated same-day runs;
- never touches `Cargo.toml` or `Cargo.lock`, and never pushes a version commit -- it tags the source commit directly;
- injects the exact tag and build kind at compile time; and
- requires the same signing and publication approvals as stable releases, then publishes a signed GitHub prerelease, keeping `/releases/latest` pointed at the last stable release.

RC and nightly publication are intentionally manual. To promote a validated RC line to stable, run the workflow again with `mode: stable` and the same `bump` level. The resulting stable tag supersedes the prerelease line; the guard blocks promotion when an RC commit is not reachable from the stable source.

## Debugging a release build

Each release includes a `strata-VERSION-TARGET.debug` file matching its stripped binary. Download the debug file for the installed version and architecture, place it beside the `strata` binary, then run `coredumpctl debug strata`; GDB follows the binary's embedded debug link to load Rust function names and source lines.

## Version calculation

The version arithmetic described above is implemented in [`scripts/release_version.py`](../scripts/release_version.py), extracted out of the workflow (it was previously an inline heredoc) so it can be unit tested. Run its tests with:

```bash
python3 scripts/test_release_version.py
```

or the way CI runs every script test in the repo:

```bash
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Cases covered:

| Case | Input | Result |
| --- | --- | --- |
| Stable patch bump | `0.5.0`, `patch`, `stable`, no tags | `0.5.1` |
| Stable minor bump | `0.5.7`, `minor`, `stable`, no tags | `0.6.0` |
| Stable major bump | `0.5.7`, `major`, `stable`, no tags | `1.0.0` |
| Stable tag collision | `0.5.0`, `patch`, `stable`, tags include `v0.5.1` | rejected |
| First alpha for a core | `0.5.0`, `patch`, `alpha`, no tags | `0.5.1-alpha.1` |
| First beta for a core | `0.5.0`, `patch`, `beta`, no tags | `0.5.1-beta.1` |
| First RC for a core with no existing RC tags | `0.5.0`, `patch`, `rc`, no tags | `0.5.1-rc.1` |
| RC after an existing RC | `0.5.0`, `patch`, `rc`, tags include `v0.5.1-rc.1` | `0.5.1-rc.2` |
| RC ordinal is numeric, not lexicographic | `0.5.0`, `patch`, `rc`, tags include `v0.5.1-rc.1` .. `v0.5.1-rc.10` | `0.5.1-rc.11` |
| RC tag collision (defense in depth) | computed tag already present | rejected |
| First nightly for a date | `0.7.0`, `minor`, `nightly`, date `20260904` | `0.8.0-nightly.20260904` |
| Repeated same-day nightly | tags end at `v0.8.0-nightly.20260904.2` | `0.8.0-nightly.20260904.3` |
| Invalid nightly calendar date | date `20260230` | rejected |
| Unrelated tags ignored | tags for other cores or build kinds mixed in | ignored |

## Known limitation

This repository has no full workflow execution harness. Version calculation and manifest signing have unit tests, including OpenSSL/Rust signature interoperability and checks on the signing job's source isolation. Those tests do not replace a real approved GitHub Actions release run for verifying environment secrets, artifact transfer, tagging, and publication.
