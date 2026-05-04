//! Path filtering: replicates the `--only` / `--ignore` / default-filetype
//! semantics from `git_of_theseus/analyze.py`.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::filetypes::default_patterns;

/// Decides whether a tree entry should be included in the analysis.
pub struct PathFilter {
    default_filetype_basename: GlobSet,
    only: Option<GlobSet>,
    ignore: Option<GlobSet>,
    all_filetypes: bool,
}

impl PathFilter {
    pub fn new(only: &[String], ignore: &[String], all_filetypes: bool) -> Result<Self> {
        let default_filetype_basename = build_globset(default_patterns().iter().copied())
            .context("building default filetype globset")?;

        let only = if only.is_empty() {
            None
        } else {
            Some(
                build_globset(only.iter().map(String::as_str))
                    .context("building --only globset")?,
            )
        };
        let ignore = if ignore.is_empty() {
            None
        } else {
            Some(
                build_globset(ignore.iter().map(String::as_str))
                    .context("building --ignore globset")?,
            )
        };

        Ok(Self {
            default_filetype_basename,
            only,
            ignore,
            all_filetypes,
        })
    }

    /// Returns true if `path` (a forward-slash repo-relative path) should be
    /// blamed.
    pub fn allows(&self, path: &str) -> bool {
        if !self.all_filetypes {
            let basename = path.rsplit('/').next().unwrap_or(path);
            if !self.default_filetype_basename.is_match(basename) {
                return false;
            }
        }
        if let Some(only) = &self.only {
            if !only.is_match(path) {
                return false;
            }
        }
        if let Some(ignore) = &self.ignore {
            if ignore.is_match(path) {
                return false;
            }
        }
        true
    }
}

fn build_globset<'a, I>(patterns: I) -> Result<GlobSet>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).with_context(|| format!("invalid glob pattern: {pat}"))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

/// Returns the top-level directory for a path, mirroring
/// `git_of_theseus/analyze.py:get_top_dir`:
///     `os.path.dirname(path).split("/")[0] + "/"`.
pub fn top_dir(path: &str) -> String {
    let dirname = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let first = dirname.split('/').next().unwrap_or("");
    format!("{first}/")
}

/// Returns the lowercase-preserving extension `.foo` of a path, including the
/// leading dot, mirroring `os.path.splitext` semantics for simple cases.
/// Returns an empty string when there is no extension.
pub fn extension(path: &str) -> String {
    let basename = path.rsplit('/').next().unwrap_or(path);
    match basename.rfind('.') {
        Some(0) => String::new(), // dotfile: ".bashrc" -> ""
        Some(idx) => basename[idx..].to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_dir_matches_python() {
        // os.path.dirname("src/foo/bar.rs") = "src/foo" -> split[0]+"/" = "src/"
        assert_eq!(top_dir("src/foo/bar.rs"), "src/");
        // For top-level files dirname is empty, so split[0]+"/" = "/".
        assert_eq!(top_dir("README.md"), "/");
        assert_eq!(top_dir("a/b"), "a/");
    }

    #[test]
    fn extension_matches_python() {
        assert_eq!(extension("foo/bar.rs"), ".rs");
        assert_eq!(extension("foo/bar"), "");
        assert_eq!(extension("foo/.bashrc"), "");
        assert_eq!(extension("foo/bar.tar.gz"), ".gz");
    }

    #[test]
    fn filter_allows_default_ext() {
        let f = PathFilter::new(&[], &[], false).unwrap();
        assert!(f.allows("src/lib.rs"));
        assert!(f.allows("a/b/c.py"));
    }

    #[test]
    fn filter_excludes_ignored_extensions() {
        let f = PathFilter::new(&[], &[], false).unwrap();
        // *.md is on the IGNORE_PYGMENTS_FILETYPES list, so excluded by default
        assert!(!f.allows("README.md"));
        // *.json likewise
        assert!(!f.allows("data/foo.json"));
    }

    #[test]
    fn filter_all_filetypes_overrides() {
        let f = PathFilter::new(&[], &[], true).unwrap();
        assert!(f.allows("README.md"));
        assert!(f.allows("data/foo.json"));
    }

    #[test]
    fn filter_only_and_ignore() {
        let f = PathFilter::new(&["src/**".to_string()], &[], false).unwrap();
        assert!(f.allows("src/lib.rs"));
        assert!(!f.allows("tests/foo.rs"));

        let f = PathFilter::new(&[], &["**/generated/**".to_string()], false).unwrap();
        assert!(f.allows("src/lib.rs"));
        assert!(!f.allows("src/generated/foo.rs"));
    }
}
