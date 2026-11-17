//! Finds which of a book's names a text has, in one pass over the text whatever the
//! number of names (Aho-Corasick), for the wiki and the cast parts of the prompts.
//!
//! A name only counts as a whole word: in a text written with spaces (Vietnamese,
//! English…) "Từ" is not found inside "Từng", nor "An" inside "Anh". Chinese, Japanese
//! and Korean names have no spaces around them, so they are found anywhere.

use aho_corasick::{AhoCorasick, MatchKind};

/// Names of several owners (an entry's name and aliases, a character…), each name
/// pointing at its owner's index.
pub struct Finder {
    ac: Option<AhoCorasick>,
    owners: Vec<usize>,
    count: usize,
}

impl Finder {
    /// `names[i]` are owner `i`'s names. Names of one letter are left out: they match
    /// everywhere.
    pub fn new<S: AsRef<str>>(names: &[Vec<S>]) -> Self {
        let mut patterns = Vec::new();
        let mut owners = Vec::new();
        for (i, list) in names.iter().enumerate() {
            for n in list {
                let n = n.as_ref().trim();
                if n.chars().count() >= 2 {
                    patterns.push(n.to_string());
                    owners.push(i);
                }
            }
        }
        let ac = (!patterns.is_empty())
            .then(|| {
                AhoCorasick::builder()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(&patterns)
                    .ok()
            })
            .flatten();
        Self {
            ac,
            owners,
            count: names.len(),
        }
    }

    /// How many times each owner is named in `text`, by owner index.
    pub fn hits(&self, text: &str) -> Vec<usize> {
        let mut hits = vec![0; self.count];
        if let Some(ac) = &self.ac {
            for m in ac.find_iter(text) {
                if whole_word(text, m.start(), m.end()) {
                    hits[self.owners[m.pattern().as_usize()]] += 1;
                }
            }
        }
        hits
    }
}

/// A letter of a script written with spaces between words, or a digit or an accent
/// mark: what may not touch a name found in such a script.
fn word_char(c: char) -> bool {
    (c.is_alphanumeric() && !no_spaces(c)) || ('\u{300}'..='\u{36F}').contains(&c)
}

/// Chinese, Japanese and Korean: names run into the words around them.
fn no_spaces(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// Whether `text[start..end]` stands as a whole word: a name that starts (ends) with a
/// letter of a spaced script has no such letter right before (after) it.
pub fn whole_word(text: &str, start: usize, end: usize) -> bool {
    let found = &text[start..end];
    let first = found.chars().next();
    let last = found.chars().next_back();
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let edge = |inner: Option<char>, outer: Option<char>| match (inner, outer) {
        (Some(i), Some(o)) => !(word_char(i) && word_char(o)),
        _ => true,
    };
    edge(first, before) && edge(last, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_owner_in_one_pass() {
        let f = Finder::new(&[vec!["林动", "小貂"], vec!["青竹"], vec!["X"]]);
        assert_eq!(f.hits("林动对小貂说，林动……"), vec![3, 0, 0]);
        assert_eq!(f.hits("青竹 X X"), vec![0, 1, 0]);
        assert_eq!(Finder::new::<&str>(&[]).hits("text"), Vec::<usize>::new());
    }

    #[test]
    fn a_name_in_a_spaced_script_is_only_a_whole_word() {
        let f = Finder::new(&[vec!["Từ"], vec!["An"], vec!["Lâm Động"]]);
        // "Từ" is not in "Từng", "An" not in "Anh"…
        assert_eq!(f.hits("Từng ngày Anh đi. Lâm Độngg"), vec![0, 0, 0]);
        // …but is a name next to spaces and punctuation.
        assert_eq!(f.hits("Từ Phong gặp An, \"Lâm Động!\""), vec![1, 1, 1]);
        // Chinese names sit right against the words around them.
        let zh = Finder::new(&[vec!["林动"]]);
        assert_eq!(zh.hits("他是林动的哥哥"), vec![1]);
    }
}
