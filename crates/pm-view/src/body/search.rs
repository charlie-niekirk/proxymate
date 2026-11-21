//! Search in bodies (`BodyViewer/searchTerms.ts`, `search.tsx`): the
//! filter's `body:` and bare-text terms (`dsl::body_search_terms`) or a
//! viewer's own ⌘F query, matched case-insensitively, counted up to
//! [`MATCH_LIMIT`], and stepped through with the match bar.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// At most this many matches are counted and stepped through.
pub const MATCH_LIMIT: usize = 10_000;

#[derive(Clone, Debug)]
pub struct Search {
    re: Regex,
}

impl Search {
    /// One case-insensitive pattern for every term (longest first, so it
    /// wins); `None` without a non-empty term.
    pub fn new<S: AsRef<str>>(terms: &[S]) -> Option<Search> {
        let mut terms: Vec<&str> = terms.iter().map(AsRef::as_ref).filter(|t| !t.is_empty()).collect();
        if terms.is_empty() {
            return None;
        }
        terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
        let pattern = terms.iter().map(|t| regex::escape(t)).collect::<Vec<_>>().join("|");
        RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .size_limit(1 << 20)
            .build()
            .ok()
            .map(|re| Search { re })
    }

    /// Where it matches in `text`, up to [`MATCH_LIMIT`].
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.ranges_up_to(text, MATCH_LIMIT)
    }

    pub fn ranges_up_to(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        self.re
            .find_iter(text)
            .filter(|m| !m.is_empty())
            .take(limit)
            .map(|m| m.range())
            .collect()
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }
}

/// The matches overlapping `window`, from `ranges` sorted by start, cut to it
/// and made relative to its start.
pub fn ranges_in(ranges: &[Range<usize>], window: Range<usize>) -> Vec<Range<usize>> {
    let first = ranges.partition_point(|r| r.end <= window.start);
    ranges[first..]
        .iter()
        .take_while(|r| r.start < window.end)
        .map(|r| r.start.max(window.start) - window.start..r.end.min(window.end) - window.start)
        .filter(|r| !r.is_empty())
        .collect()
}

/// "No matches", "12 matches", "10,000+ matches", "3 of 12".
pub fn match_label(count: usize, current: Option<usize>) -> String {
    match current {
        _ if count == 0 => "No matches".into(),
        None if count >= MATCH_LIMIT => format!("{}+ matches", super::grouped(MATCH_LIMIT as u64)),
        None => format!(
            "{} match{}",
            super::grouped(count as u64),
            if count == 1 { "" } else { "es" }
        ),
        Some(i) => format!("{} of {}", i + 1, super::grouped(count as u64)),
    }
}

/// The match a step lands on: the first (or last) one, then round.
pub fn step(current: Option<usize>, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        None if forward => 0,
        None => count - 1,
        Some(i) if forward => (i + 1) % count,
        Some(i) => (i + count - 1) % count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_every_term_case_insensitively_longest_first() {
        let s = Search::new(&["id", "user_id", ""]).unwrap();
        assert_eq!(s.ranges("USER_ID and Id"), [0..7, 12..14]);
        assert!(Search::new::<&str>(&[]).is_none());
        assert!(
            Search::new(&["a.b"]).unwrap().ranges("axb").is_empty(),
            "terms are literal"
        );
        assert_eq!(Search::new(&["é"]).unwrap().ranges("É"), vec![0..2]);
    }

    #[test]
    fn cuts_ranges_to_a_window() {
        let ranges = [0..3, 5..9, 12..14];
        assert_eq!(ranges_in(&ranges, 4..8), vec![1..4]);
        assert_eq!(ranges_in(&ranges, 8..13), [0..1, 4..5]);
        assert!(ranges_in(&ranges, 15..20).is_empty());
    }

    #[test]
    fn labels_and_steps() {
        assert_eq!(match_label(0, None), "No matches");
        assert_eq!(match_label(1, None), "1 match");
        assert_eq!(match_label(MATCH_LIMIT, None), "10,000+ matches");
        assert_eq!(match_label(12, Some(2)), "3 of 12");
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(None, 3, false), Some(2));
        assert_eq!(step(Some(2), 3, true), Some(0));
        assert_eq!(step(Some(0), 3, false), Some(2));
        assert_eq!(step(None, 0, true), None);
    }
}
