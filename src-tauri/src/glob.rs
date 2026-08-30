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

/// The user's exclude patterns, compiled once.
///
/// This used to be a free `matches(rel_path, &[String])` that called
/// `Regex::new` for every pattern on every path it was asked about — from
/// the source walk *and* again from the prune pass. A 100k-file tree with
/// five patterns meant a million regex compilations per run, all of them
/// producing the same handful of automata.
pub struct PatternSet {
    /// (compiled pattern, is a `!` re-include)
    rules: Vec<(regex::Regex, bool)>,
}

impl PatternSet {
    pub fn new(patterns: &[String]) -> Self {
        let rules = patterns
            .iter()
            .filter_map(|pattern| {
                let (body, negated) = match pattern.strip_prefix('!') {
                    Some(body) => (body, true),
                    None => (pattern.as_str(), false),
                };
                // `matches` normalises the candidate path to `/` before
                // testing, so the pattern has to agree. Without this the
                // separator a Windows user reaches for first was escaped into
                // a *literal* backslash — `Photos\raw` compiled to
                // `^Photos\\raw$`, matched nothing against `Photos/raw`, and
                // was never reported as wrong. The folder they thought they
                // had excluded was copied on every run.
                let body = body.replace('\\', "/");
                // A pattern that doesn't compile is dropped rather than
                // failing the run, matching the pre-1.5 behaviour.
                // NTFS and APFS filenames are case-insensitive, and this same
                // set is consulted by the source walk *and* by the prune pass.
                // A case-sensitive pattern agrees with itself only as long
                // as both sides see the same spelling: re-case a folder in
                // the source and `raw` stops it being copied while `RAW` at
                // the destination no longer looks protected, so prune wipes
                // the subtree (#R6).
                regex::RegexBuilder::new(&glob_to_regex(&body))
                    .case_insensitive(crate::fsutil::CASE_INSENSITIVE_FS)
                    .build()
                    .ok()
                    .map(|re| (re, negated))
            })
            .collect();
        Self { rules }
    }

    pub fn from_input(input: &str) -> Self {
        Self::new(&parse_patterns(input))
    }

    /// True if `rel_path` should be excluded. Each rule is tried against the
    /// full relative path *and* against the basename — so `*.tmp` excludes
    /// `a/b/c.tmp`, which is the user-friendly reading — and the last rule
    /// to match wins, which is what makes `!keep.tmp` re-include.
    pub fn matches(&self, rel_path: &str) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        let normalized = rel_path.replace('\\', "/");
        let basename = normalized.rsplit('/').next().unwrap_or("");
        let mut excluded = false;
        for (re, negated) in &self.rules {
            if re.is_match(&normalized) || re.is_match(basename) {
                excluded = !negated;
            }
        }
        excluded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same PatternSet is consulted by the source walk and by the prune
    /// pass, which see the source and destination spellings respectively.
    /// On NTFS and APFS those differ after a re-case, and a case-sensitive
    /// match meant prune deleted a subtree the user had excluded (#R6).
    ///
    /// Asserted against the constant rather than skipped off-Windows, so the
    /// Linux job also pins the case-*sensitive* half of the contract: there,
    /// `RAW` genuinely is a different folder and must not be excluded by a
    /// pattern that says `raw`.
    #[test]
    fn patterns_ignore_case_exactly_where_the_filesystem_does() {
        let set = PatternSet::from_input("raw");
        assert!(set.matches("raw"), "the source spelling excludes");
        let folds = crate::fsutil::CASE_INSENSITIVE_FS;
        assert_eq!(
            set.matches("RAW"),
            folds,
            "the destination spelling must too, on a folding filesystem"
        );
        assert_eq!(set.matches("Photos/Raw"), folds, "and at depth");
    }

    fn set(patterns: &[&str]) -> PatternSet {
        PatternSet::new(&patterns.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    /// The separator a Windows user reaches for first has to work. It used to
    /// be escaped into a *literal* backslash, so `Photos\raw` compiled to
    /// `^Photos\\raw$` and could never match the `Photos/raw` the walk
    /// produces. Nothing reported the rule as wrong — it simply excluded
    /// nothing, and the folder was copied on every run.
    #[test]
    fn a_pattern_written_with_backslashes_still_excludes() {
        let backslash = char::from(92u8);
        let win = format!("Photos{}raw", backslash);
        assert!(set(&[&win]).matches("Photos/raw"));
        // And the same rule reaches the same path however the caller spells it.
        assert!(set(&[&win]).matches(&format!("Photos{}raw", backslash)));
        // The forward-slash spelling keeps working, unchanged.
        assert!(set(&["Photos/raw"]).matches("Photos/raw"));
    }

    /// Mixed separators in one pattern, which is what a pasted path looks
    /// like, and a negation written the same way.
    #[test]
    fn backslashes_work_at_depth_and_under_negation() {
        let b = char::from(92u8);
        assert!(set(&[&format!("a{}b/c", b)]).matches("a/b/c"));
        let p = set(&[&format!("node_modules{}*", b), &format!("!node_modules{}keep", b)]);
        assert!(p.matches("node_modules/junk"));
        assert!(!p.matches("node_modules/keep"));
    }

    #[test]
    fn parses_separators() {
        assert_eq!(
            parse_patterns("*.tmp, node_modules\n.git"),
            vec!["*.tmp", "node_modules", ".git"]
        );
    }

    #[test]
    fn basename_match() {
        assert!(set(&["*.tmp"]).matches("foo/bar.tmp"));
        assert!(!set(&["*.tmp"]).matches("foo/bar.txt"));
    }

    #[test]
    fn negation_reincludes() {
        let p = set(&["*.tmp", "!keep.tmp"]);
        assert!(p.matches("skip.tmp"));
        assert!(!p.matches("keep.tmp"));
    }

    #[test]
    fn last_matching_rule_wins() {
        // Order matters in both directions — a re-include can itself be
        // overridden by a later exclude.
        let p = set(&["*.tmp", "!keep.tmp", "keep.tmp"]);
        assert!(p.matches("keep.tmp"));
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
        assert!(!set(&[]).matches("any/path/here.txt"));
    }

    #[test]
    fn double_star_crosses_directories() {
        // `**` matches across path separators in the full-rel-path check.
        assert!(set(&["**/*.tmp"]).matches("a/b/c.tmp"));
        assert!(set(&["**/file.log"]).matches("deep/nested/file.log"));
        // A bare `*` glob without `**` only matches a single segment when
        // checked against the full rel-path, but our matcher also tries
        // the *basename*, so `*.tmp` still excludes `a/b/c.tmp` via the
        // basename `c.tmp` — that's the user-friendly default.
        assert!(set(&["*.tmp"]).matches("a/b/c.tmp"));
    }

    #[test]
    fn question_mark_is_single_non_slash() {
        assert!(set(&["?.b"]).matches("a.b"));
        // `?` must not match `/`, otherwise an exclude like `a?b` would
        // accidentally swallow `a/b`.
        assert!(!set(&["a?b"]).matches("a/b"));
    }

    #[test]
    fn special_regex_chars_are_escaped() {
        // A literal `.` in a glob should match a literal `.`, not "any char".
        assert!(set(&["file.txt"]).matches("file.txt"));
        assert!(!set(&["file.txt"]).matches("fileXtxt"));
    }

    #[test]
    fn from_input_matches_parse_then_new() {
        let input = "*.tmp,\n!keep.tmp";
        let a = PatternSet::from_input(input);
        let b = PatternSet::new(&parse_patterns(input));
        for path in ["skip.tmp", "keep.tmp", "a/b/other.txt"] {
            assert_eq!(a.matches(path), b.matches(path), "diverged on {}", path);
        }
    }
}
