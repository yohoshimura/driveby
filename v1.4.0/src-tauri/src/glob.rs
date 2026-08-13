pub fn parse_patterns(input: &str) -> Vec<String> {
    input
        .split([',', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn glob_to_regex(glob: &str) -> String {
    let mut re = String::from("^");
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    re.push_str(".*");
                } else {
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push_str("[^/]"),
            '.' | '+' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            other => re.push(other),
        }
    }
    re.push('$');
    re
}

pub fn matches(rel_path: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let normalized = rel_path.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or("");
    let mut excluded = false;
    for pattern in patterns {
        let (body, negated) = if let Some(body) = pattern.strip_prefix('!') {
            (body, true)
        } else {
            (pattern.as_str(), false)
        };
        let re = regex::Regex::new(&glob_to_regex(body));
        let Ok(re) = re else { continue };
        if re.is_match(&normalized) || re.is_match(basename) {
            excluded = !negated;
        }
    }
    excluded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_separators() {
        assert_eq!(
            parse_patterns("*.tmp, node_modules\n.git"),
            vec!["*.tmp", "node_modules", ".git"]
        );
    }

    #[test]
    fn basename_match() {
        assert!(matches("foo/bar.tmp", &["*.tmp".into()]));
        assert!(!matches("foo/bar.txt", &["*.tmp".into()]));
    }

    #[test]
    fn negation_reincludes() {
        let p = vec!["*.tmp".into(), "!keep.tmp".into()];
        assert!(matches("skip.tmp", &p));
        assert!(!matches("keep.tmp", &p));
    }

    #[test]
    fn parse_patterns_drops_blank_and_whitespace_only() {
        // Blanks, whitespace-only segments, and trailing separators must
        // not produce empty pattern entries (which would otherwise compile
        // to `^$` and match every empty rel-path basename).
        let parsed = parse_patterns("  ,*.log,   ,\n\nnode_modules,\n");
        assert_eq!(parsed, vec!["*.log", "node_modules"]);
    }

    #[test]
    fn empty_pattern_list_matches_nothing() {
        // The early-return in matches() avoids ever feeding "" to glob_to_regex.
        assert!(!matches("any/path/here.txt", &[]));
    }

    #[test]
    fn double_star_crosses_directories() {
        // `**` matches across path separators in the full-rel-path check.
        assert!(matches("a/b/c.tmp", &["**/*.tmp".into()]));
        assert!(matches("deep/nested/file.log", &["**/file.log".into()]));
        // A bare `*` glob without `**` only matches a single segment when
        // checked against the full rel-path, but our matcher also tries
        // the *basename*, so `*.tmp` still excludes `a/b/c.tmp` via the
        // basename `c.tmp` — that's the user-friendly default.
        assert!(matches("a/b/c.tmp", &["*.tmp".into()]));
    }

    #[test]
    fn question_mark_is_single_non_slash() {
        assert!(matches("a.b", &["?.b".into()]));
        // `?` must not match `/`, otherwise an exclude like `a?b` would
        // accidentally swallow `a/b`.
        assert!(!matches("a/b", &["a?b".into()]));
    }

    #[test]
    fn special_regex_chars_are_escaped() {
        // A literal `.` in a glob should match a literal `.`, not "any char".
        assert!(matches("file.txt", &["file.txt".into()]));
        assert!(!matches("fileXtxt", &["file.txt".into()]));
    }
}
