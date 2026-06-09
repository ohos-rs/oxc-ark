use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub fn build_ignore_matcher(root: &Path, patterns: &[String]) -> Result<Option<Gitignore>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        if builder.add_line(None, pattern).is_err() {
            return Err(format!(
                "Failed to add ignore pattern `{pattern}` from `ignorePatterns`"
            ));
        }
    }
    builder
        .build()
        .map(Some)
        .map_err(|_| "Failed to build ignores".to_string())
}

pub fn is_gitignore_match(
    matcher: &Gitignore,
    path: &Path,
    is_dir: bool,
    check_ancestors: bool,
) -> bool {
    if check_ancestors {
        if !path.starts_with(matcher.path()) {
            return false;
        }
        return matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore();
    }

    matcher.matched(path, is_dir).is_ignore()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{build_ignore_matcher, is_gitignore_match};

    #[test]
    fn directory_pattern_matches_descendants() {
        let root = Path::new("/repo");
        let matcher = build_ignore_matcher(root, &["dist".to_string()])
            .expect("ignore matcher should build")
            .expect("patterns should produce matcher");

        assert!(is_gitignore_match(
            &matcher,
            Path::new("/repo/dist/input.ts"),
            false,
            true
        ));
        assert!(!is_gitignore_match(
            &matcher,
            Path::new("/repo/src/input.ts"),
            false,
            true
        ));
    }

    #[test]
    fn outside_root_is_not_ignored() {
        let root = Path::new("/repo");
        let matcher = build_ignore_matcher(root, &["dist".to_string()])
            .expect("ignore matcher should build")
            .expect("patterns should produce matcher");

        assert!(!is_gitignore_match(
            &matcher,
            Path::new("/other/dist/input.ts"),
            false,
            true
        ));
    }
}
