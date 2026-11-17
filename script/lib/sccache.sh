# sccache, when on PATH, as the compiler wrapper. Sourced by script/test and script/mutants: the
# dependencies compiled in one checkout, a worktree or a copy that cargo-mutants makes, come from its
# cache in the next. Without it, cargo compiles as it would.

# sccache_use: exports RUSTC_WRAPPER=sccache if sccache is on PATH.
sccache_use() {
  if command -v sccache >/dev/null; then
    export RUSTC_WRAPPER=sccache
  fi
}
