//! Tuning of libgit2's global object cache.
//!
//! libgit2 caches parsed commits/trees/tags up to 4 KiB by default, but its
//! blob cache limit is **0**, meaning blobs are never cached: every
//! historical blob touched during `blame` gets re-read from the odb and
//! re-inflated, even when the same blob is revisited across sampled
//! commits. Raising the per-blob size eligible for caching lets libgit2
//! reuse already-inflated blobs (bounded by the existing global
//! `GIT_OPT_SET_CACHE_MAX_SIZE`, 256 MiB by default), which measurably
//! speeds up blame-heavy workloads.
//!
//! `git2` has no safe wrapper for `GIT_OPT_SET_CACHE_OBJECT_LIMIT`, so this
//! calls into `libgit2-sys` directly (pinned to the exact version `git2`
//! uses, see the workspace `Cargo.toml`).

use std::os::raw::c_int;
use std::sync::Once;

/// Blob size (in bytes) up to which libgit2 will keep a parsed blob in its
/// global object cache. Chosen to cover the vast majority of source files
/// while staying well under the 256 MiB total cache budget.
const BLOB_CACHE_OBJECT_LIMIT: usize = 1024 * 1024;

static INIT: Once = Once::new();

/// Raises libgit2's blob object-cache limit so that blobs re-visited across
/// sampled commits during `blame` are served from the in-memory cache
/// instead of being re-read and re-inflated from the odb.
///
/// This mutates process-global libgit2 state and is not safe to call
/// concurrently with other libgit2 calls, so callers must invoke it once,
/// before spawning any worker threads that touch libgit2 (e.g. before
/// building the rayon thread pool). It is safe to call multiple times: only
/// the first call takes effect.
pub fn configure_blob_object_cache() {
    INIT.call_once(|| {
        let error = set_cache_object_limit(BLOB_CACHE_OBJECT_LIMIT);
        // `GIT_OPT_SET_CACHE_OBJECT_LIMIT` cannot actually fail for a valid
        // object type/size (mirrors `git2::opts::enable_caching`'s
        // `debug_assert!` on the same FFI call), but surface a mismatch
        // loudly in debug builds rather than silently leaving the blob
        // cache disabled.
        debug_assert!(
            error >= 0,
            "git_libgit2_opts(SET_CACHE_OBJECT_LIMIT) failed: {error}"
        );
    });
}

/// Raw FFI call, split out from [`configure_blob_object_cache`] so tests can
/// assert on libgit2's return code directly.
fn set_cache_object_limit(limit: usize) -> c_int {
    // Safety: this only mutates the plain, process-global size limit
    // libgit2 checks before caching a parsed object (it has no dependency
    // on `git_libgit2_init` having run yet). Callers of
    // `configure_blob_object_cache` are required to invoke it before
    // spawning any worker threads that touch libgit2, so no other thread
    // is concurrently touching libgit2's global cache state.
    unsafe {
        libgit2_sys::git_libgit2_opts(
            libgit2_sys::GIT_OPT_SET_CACHE_OBJECT_LIMIT as c_int,
            libgit2_sys::GIT_OBJECT_BLOB as c_int,
            limit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_blob_object_cache_is_idempotent() {
        // Calling this repeatedly (e.g. across multiple `#[test]` fns in
        // the same process, or multiple `analyze_many` calls) must not
        // panic or otherwise misbehave; only the first call should take
        // effect.
        configure_blob_object_cache();
        configure_blob_object_cache();
    }

    #[test]
    fn set_cache_object_limit_succeeds() {
        // Exercises the actual FFI call (bypassing the `Once` guard) and
        // asserts libgit2 reports success, rather than only checking that
        // the wrapper doesn't panic.
        assert!(set_cache_object_limit(BLOB_CACHE_OBJECT_LIMIT) >= 0);
    }
}
