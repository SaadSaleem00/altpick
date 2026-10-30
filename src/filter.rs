//! Type-to-filter matching.
//!
//! Ranking: names that start with the query, then names where it starts a
//! word ("my-app.js" for "app"), then names that contain it anywhere, then
//! names whose letters appear in order ("ap" matches "a-p-p.js").
//! Ties keep the folder's normal order.

use crate::list::Entry;

/// Returns indexes into `entries` that match `query`, best first.
pub fn apply(entries: &[Entry], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let q: Vec<char> = query.chars().collect();
    let mut scored: Vec<(i64, usize)> =
        entries.iter().enumerate().filter_map(|(i, e)| score(&query, &q, &e.lower).map(|s| (s, i))).collect();
    // Stable sort keeps the original order for equal scores.
    scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
    scored.into_iter().map(|(_, i)| i).collect()
}

fn score(query: &str, q: &[char], name: &str) -> Option<i64> {
    if name.starts_with(query) {
        return Some(3_000);
    }
    // Contains the query: better if it starts a word ("my-app", "app.js" for "js").
    let mut best = None;
    for (pos, _) in name.match_indices(query) {
        let at_word_start = name[..pos].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let s = if at_word_start { 2_500 } else { 2_000 };
        best = best.max(Some(s));
    }
    if best.is_some() {
        return best;
    }
    subsequence(q, name).map(|span| 1_000 - (span as i64).min(999))
}

/// If every query char appears in order, returns how spread out the match is.
fn subsequence(q: &[char], name: &str) -> Option<usize> {
    let mut qi = 0;
    let mut first = None;
    let mut last = 0;
    for (i, c) in name.chars().enumerate() {
        if qi < q.len() && c == q[qi] {
            first.get_or_insert(i);
            last = i;
            qi += 1;
        }
    }
    if qi == q.len() {
        Some(last - first.unwrap_or(0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[&str]) -> Vec<Entry> {
        names.iter().map(|n| Entry::new(n.to_string(), false)).collect()
    }

    fn pick<'a>(e: &'a [Entry], q: &str) -> Vec<&'a str> {
        apply(e, q).into_iter().map(|i| e[i].name.as_str()).collect()
    }

    #[test]
    fn empty_query_keeps_everything_in_order() {
        let e = entries(&["b.js", "a.js"]);
        assert_eq!(pick(&e, ""), ["b.js", "a.js"]);
    }

    #[test]
    fn prefix_beats_contains_beats_scattered() {
        let e = entries(&["my-app.js", "zap.txt", "app.js", "a_x_p_p.js", "readme.md"]);
        assert_eq!(pick(&e, "ap"), ["app.js", "my-app.js", "zap.txt", "a_x_p_p.js"]);
    }

    #[test]
    fn extension_query_keeps_folder_order() {
        let e = entries(&["app.js", "file2.js", "notes.md", "server.js", "漢字.js"]);
        assert_eq!(pick(&e, "js"), ["app.js", "file2.js", "server.js", "漢字.js"]);
    }

    #[test]
    fn case_insensitive_and_unicode() {
        let e = entries(&["Server.JS", "رپورٹ.txt"]);
        assert_eq!(pick(&e, "server"), ["Server.JS"]);
        assert_eq!(pick(&e, "رپو"), ["رپورٹ.txt"]);
    }

    #[test]
    fn no_match() {
        let e = entries(&["index.js"]);
        assert!(pick(&e, "zz").is_empty());
    }
}
