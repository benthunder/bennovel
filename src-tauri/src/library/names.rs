//! Finds which of a book's names a text has, in one pass over the text whatever the
//! number of names (Aho-Corasick), for the wiki and the cast parts of the prompts.

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
                hits[self.owners[m.pattern().as_usize()]] += 1;
            }
        }
        hits
    }
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
}
