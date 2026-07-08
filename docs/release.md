# Release runbook

shirobai releases all 4 gems together with one shared CalVer version
(`YYYY.MMDD.HHMM`, JST): `shirobai` (core), `shirobai-performance`,
`shirobai-rspec`, `shirobai-rails`.

The plugin gemspecs derive their core pin from their own `VERSION`
constant (`spec.add_dependency "shirobai", "= #{...::VERSION}"`),
so keeping the 4 `version.rb` files equal keeps the whole set in lockstep.
Never release a subset — a version gap between the gems reads as breakage.

## One-time prerequisite

Each gem must be registered as a **Trusted Publisher** on rubygems.org
for this repository and the `release.yml` workflow.
`shirobai` (core) is registered from its first release; the 3 plugin gems
need the same registration before their first release.
Without it, `gem push` for that gem is rejected.

## Release flow

1. **Dispatch the bump workflow**:

   ```sh
   gh workflow run bump-version.yml
   ```

   It generates the CalVer string and opens a release PR that changes
   all 4 `version.rb` files to the same value.

2. **Review and merge the release PR.**
   The merge is the human gate. Check that all 4 files carry the same
   version string.

3. **`release.yml` does the rest** — it fires on the push to main that
   changes `lib/shirobai/version.rb` (core path only; the other 3 files
   arrive in the same merge), tags `v<version>`, and runs `rake release`
   through `rubygems/release-gem` (OIDC trusted publishing).
   `rake release` builds the 4 gems into `pkg/` and pushes **core first**,
   waits for the index to serve it, then pushes the 3 plugins
   (their `= version` dependency must resolve against the new core).

4. **Verify**: the 4 rubygems.org pages show the new version, and

   ```sh
   gem install shirobai shirobai-rspec -v <version>
   ```

   resolves cleanly on a machine with a Rust toolchain.

## Local rehearsal (no push)

```sh
rake build     # builds all 4 gems into pkg/
gem spec pkg/shirobai-rspec-<version>.gem dependencies
```

Check that each plugin gem pins `shirobai = <version>` and its stock
plugin gem at the pinned version.

## If something goes wrong

- A failed plugin push after a successful core push leaves a partial
  release. **Use "Re-run failed jobs" on the release run first**:
  the tag step skips an existing tag, and `rake release` skips gem
  versions that are already on RubyGems.org, so a re-run only does
  the missing pushes (the version is already burned; do not re-bump
  for a partial failure).
- If the workflow path is broken itself, push the built gems by hand:
  `gem push pkg/<gem>` from a `rake build` checkout of the release tag.
  A gem first published by hand has no trusted publisher; add one on
  the gem's Settings page afterwards or the next workflow push fails.
- A bad release is yanked per gem (`gem yank <name> -v <version>`);
  yank all 4 so the set stays consistent, then bump again.

## Lessons from the first 4-gem release (2026.0708.2200)

- `gem exec` dies under `bundle exec rake`: Bundler blocks gems that
  are not in the Gemfile. The await call now runs inside
  `Bundler.with_unbundled_env`.
- `gem install shirobai` was broken even though every CI gate was
  green: the RubyGems Cargo builder derives the dylib name from the
  **package name** in `ext/shirobai/Cargo.toml`, and ours (`shirobai-ext`)
  did not match the `[lib]` name (`shirobai`). CI never caught it
  because every CI path uses the repo checkout, not an installed gem.
  The package is now named `shirobai`. Keep an install smoke test in
  the release flow: `gem install <built core gem>` into a scratch
  `GEM_HOME` exercises the path that CI does not.

## Future work (deferred on purpose)

Prebuilt platform gems (`arm64-darwin`, `x86_64-linux`) via
rb-sys + `oxidize-rb/actions/cross-gem` are designed but deferred —
they require migrating the core gem from the RubyGems Cargo builder to
`extconf.rb` + `rb_sys/mkmf` first. Until then, every install builds
from source and needs a Rust toolchain.
