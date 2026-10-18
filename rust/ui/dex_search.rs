// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared fuzzy matching for the Essentials explorers.
pub(super) fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            'é' | 'è' | 'ê' => Some('e'),
            'á' | 'à' | 'ä' => Some('a'),
            'í' | 'ï' => Some('i'),
            'ó' | 'ö' => Some('o'),
            'ú' | 'ü' => Some('u'),
            'ñ' => Some('n'),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Rank exact, prefix and substring matches ahead of subsequences and typos.
pub(super) fn fuzzy_score(query: &str, candidate: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    if query == candidate {
        return Some(0);
    }
    if candidate.starts_with(query) {
        return Some(10 + candidate.len() - query.len());
    }
    if let Some(offset) = candidate.find(query) {
        return Some(30 + offset);
    }
    let mut chars = candidate.chars();
    if query
        .chars()
        .all(|needle| chars.by_ref().any(|c| c == needle))
    {
        return Some(60 + candidate.len().saturating_sub(query.len()));
    }
    if query.chars().count() < 4 {
        return None;
    }
    let target: Vec<_> = candidate.chars().collect();
    let mut previous: Vec<_> = (0..=target.len()).collect();
    for (i, a) in query.chars().enumerate() {
        let mut row = vec![i + 1];
        for (j, b) in target.iter().enumerate() {
            row.push(
                (row[j] + 1)
                    .min(previous[j + 1] + 1)
                    .min(previous[j] + usize::from(a != *b)),
            );
        }
        previous = row;
    }
    let distance = previous[target.len()];
    (distance <= 2).then_some(100 + distance)
}
