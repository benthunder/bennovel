//! Finds chapter headings in plain text: "Chương 12: Gặp lại", "Chapter IV",
//! "第十二章 重逢", "Глава 3", "Lời mở đầu"… Each line is first matched on its own,
//! then the candidates are filtered as a whole (a table of contents at the start,
//! sentences that merely begin with "Chương 3").

/// Lines longer than this (in characters) are never headings.
pub const MAX_HEADING_CHARS: usize = 80;
/// Headings closer than this (bytes of text between them) are a list, not chapters.
pub const MIN_GAP_BYTES: u64 = 200;
/// Fewer headings than this: the text has no usable chapters.
pub const MIN_CHAPTERS: usize = 3;

/// Words followed by a number that start a chapter (or volume) heading, lowercase.
const NUMBERED: &[&str] = &[
    "chương",
    "chuong",
    "hồi",
    "hoi",
    "quyển",
    "quyen",
    "phần",
    "phan",
    "tập",
    "tap",
    "chapter",
    "chap",
    "chap.",
    "ch.",
    "part",
    "volume",
    "vol.",
    "vol",
    "book",
    "episode",
    "глава",
    "часть",
    "том",
];

/// Headings without a number ("Lời mở đầu", "Prologue", "楔子"), lowercase.
const NAMED: &[&str] = &[
    "lời mở đầu",
    "mở đầu",
    "phần mở đầu",
    "lời tựa",
    "lời nói đầu",
    "tiết tử",
    "dẫn nhập",
    "phần kết",
    "vĩ thanh",
    "lời kết",
    "hậu ký",
    "ngoại truyện",
    "phiên ngoại",
    "prologue",
    "epilogue",
    "interlude",
    "afterword",
    "preface",
    "foreword",
    "пролог",
    "эпилог",
    "序章",
    "序言",
    "楔子",
    "引子",
    "前言",
    "尾声",
    "尾聲",
    "后记",
    "後記",
    "番外",
];

/// Vietnamese number words ("Chương mười hai" is rare, "Phần hai" is not).
const VI_NUMBERS: &[&str] = &[
    "một", "nhất", "hai", "ba", "bốn", "tư", "năm", "lăm", "sáu", "bảy", "tám", "chín", "mười",
    "mươi", "mốt", "trăm", "linh", "cuối",
];

/// Characters drawn around headings ("=== Chương 1 ===", "【第一章】").
const DECOR: &[char] = &[
    '=', '*', '#', '-', '_', '~', '·', '•', '【', '】', '[', ']', '「', '」', '『', '』', '《',
    '》', '〖', '〗', '(', ')', '（', '）', '<', '>',
];

/// A line that looks like a chapter heading, with what the filters need to know.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Byte offset where the line starts.
    pub start: u64,
    /// Byte offset just past the line (after its line break).
    pub end: u64,
    /// The heading without surrounding whitespace and decoration.
    pub label: String,
    /// The line starts with whitespace (body paragraphs usually do, headings not).
    pub indented: bool,
    /// The line ends like a sentence.
    pub ends_sentence: bool,
}

/// `line` (one line, any surrounding whitespace) as a heading label, or None.
pub fn heading(line: &str) -> Option<String> {
    let label = line
        .trim_matches(|c: char| c.is_whitespace() || DECOR.contains(&c))
        .trim();
    if label.is_empty() || label.chars().count() > MAX_HEADING_CHARS {
        return None;
    }
    let ok = han_numbered(label) || word_numbered(label) || named(label);
    ok.then(|| label.to_string())
}

/// The candidate for a line, when it reads like a heading.
pub fn candidate(line: &str, start: u64, end: u64) -> Option<Candidate> {
    let label = heading(line)?;
    let trimmed = line.trim_end();
    Some(Candidate {
        start,
        end,
        indented: line.starts_with(char::is_whitespace),
        ends_sentence: trimmed.ends_with([
            '.', '!', '?', ',', ';', '。', '！', '？', '，', '；', '…', '"', '”',
        ]),
        label,
    })
}

/// 第12章 / 第 十二 章 / 第一百零五回 / 第3卷, then anything.
fn han_numbered(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('第') else {
        return false;
    };
    let rest = rest.trim_start();
    let num_len: usize = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || c.is_whitespace() || is_han_digit(*c))
        .map(char::len_utf8)
        .sum();
    let num = rest[..num_len].trim();
    !num.is_empty()
        && rest[num_len..].starts_with(['章', '节', '節', '回', '卷', '集', '部', '篇', '话', '話'])
}

fn is_han_digit(c: char) -> bool {
    "零〇一二两兩三四五六七八九十百千万萬".contains(c)
}

/// Separators between a chapter word and its number.
const SEP: &[char] = &[' ', '\u{a0}', '\u{3000}', '.', ':', '_', '#'];

/// A chapter word, a number (digits, Roman, Chinese or Vietnamese number words),
/// then the end of the line or a separator before the title.
fn word_numbered(s: &str) -> bool {
    let lower = s.to_lowercase();
    for word in NUMBERED {
        let Some(after) = lower.strip_prefix(word) else {
            continue;
        };
        // "chapter" must not match "chapters", "tập" not "tập trung".
        if !word.ends_with('.')
            && !after.starts_with(|c: char| c.is_ascii_digit() || SEP.contains(&c))
        {
            continue;
        }
        let after = after.trim_start_matches(SEP);
        // "Hồi thứ nhất", "Chương thứ hai".
        let after = after.strip_prefix("thứ ").unwrap_or(after);
        if number_end(after).is_some() {
            return true;
        }
    }
    false
}

/// Whether `s` (lowercase) starts with a number that ends the line or is followed by
/// a title: Some(byte length of the number).
fn number_end(s: &str) -> Option<usize> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let len = if digits > 0 {
        // "12.5" and "12-13" stay one number.
        let extra = s[digits..]
            .strip_prefix(['.', '-'])
            .map(|r| r.bytes().take_while(u8::is_ascii_digit).count())
            .filter(|&n| n > 0)
            .map_or(0, |n| n + 1);
        digits + extra
    } else {
        let roman = s.bytes().take_while(|b| b"ivxlcdm".contains(b)).count();
        let han: usize = s
            .chars()
            .take_while(|&c| is_han_digit(c))
            .map(char::len_utf8)
            .sum();
        if (1..=8).contains(&roman) && !s[roman..].starts_with(char::is_alphanumeric) {
            roman
        } else if han > 0 {
            han
        } else {
            // Number words are ordinary words too ("Phần một số người…"): they must
            // end the line or be followed by punctuation.
            let words = vi_number_len(s)?;
            let tail = s[words..].trim_start();
            return (tail.is_empty() || tail.starts_with(|c: char| !c.is_alphanumeric()))
                .then_some(words);
        }
    };
    let tail = &s[len..];
    (tail.is_empty() || tail.starts_with(|c: char| !c.is_alphanumeric())).then_some(len)
}

/// Byte length of the Vietnamese number words at the start of `s` ("mười hai").
fn vi_number_len(s: &str) -> Option<usize> {
    let mut len = 0;
    loop {
        let rest = &s[len..];
        let word_len: usize = rest
            .chars()
            .take_while(|c| c.is_alphabetic())
            .map(char::len_utf8)
            .sum();
        let word = &rest[..word_len];
        if !VI_NUMBERS.contains(&word) {
            // Undo the space taken before a word that is not a number.
            len -= usize::from(len > 0);
            break;
        }
        len += word.len();
        // Another number word may follow after one space.
        if !s[len..].starts_with(' ') {
            break;
        }
        len += 1;
    }
    (len > 0).then_some(len)
}

/// "Lời mở đầu", "Prologue: …", "番外一 …": the word alone or followed by a
/// separator; Chinese titles follow without one.
fn named(s: &str) -> bool {
    let lower = s.to_lowercase();
    NAMED.iter().any(|w| {
        lower.strip_prefix(w).is_some_and(|rest| {
            let han = w.chars().all(|c| c > '\u{2e80}');
            let rest = rest.trim_start();
            han || rest.is_empty() || rest.starts_with([':', '：', '-', '–', '—', '.'])
        })
    })
}

/// Keeps the candidates that are real chapter starts, in order; empty when fewer
/// than [`MIN_CHAPTERS`] remain.
///
/// - Indented lines and lines ending like a sentence are dropped when they are the
///   exception (under one in five candidates): those are story text.
/// - Headings with almost no text between them: a run of three or more is a table
///   of contents and is dropped entirely; otherwise only the last of the run is kept
///   ("Quyển 1" right before "Chương 1").
pub fn filter(mut cands: Vec<Candidate>) -> Vec<Candidate> {
    let total = cands.len();
    let indented = cands.iter().filter(|c| c.indented).count();
    if indented * 5 < total {
        cands.retain(|c| !c.indented);
    }
    let total = cands.len();
    let sentences = cands.iter().filter(|c| c.ends_sentence).count();
    if sentences * 5 < total {
        cands.retain(|c| !c.ends_sentence);
    }

    let mut kept = Vec::with_capacity(cands.len());
    let mut run: Vec<Candidate> = Vec::new();
    for c in cands {
        let close = run
            .last()
            .is_some_and(|prev| c.start.saturating_sub(prev.end) < MIN_GAP_BYTES);
        if !close {
            end_run(&mut run, &mut kept);
        }
        run.push(c);
    }
    end_run(&mut run, &mut kept);

    if kept.len() < MIN_CHAPTERS {
        kept.clear();
    }
    kept
}

fn end_run(run: &mut Vec<Candidate>, kept: &mut Vec<Candidate>) {
    if run.len() < 3 {
        kept.extend(run.pop());
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_common_headings() {
        for (line, label) in [
            ("Chương 12: Gặp lại", "Chương 12: Gặp lại"),
            ("  CHƯƠNG 1 ", "CHƯƠNG 1"),
            ("Chuong 3 - Mua dem", "Chuong 3 - Mua dem"),
            ("=== Chương 7 ===", "Chương 7"),
            ("Hồi thứ nhất", "Hồi thứ nhất"),
            ("Chương mười hai", "Chương mười hai"),
            ("Chương cuối: Tạm biệt", "Chương cuối: Tạm biệt"),
            ("Quyển 2", "Quyển 2"),
            ("Phần hai: Trở về", "Phần hai: Trở về"),
            ("Chapter IV", "Chapter IV"),
            ("Chapter 12.5 Extra", "Chapter 12.5 Extra"),
            ("Ch. 3", "Ch. 3"),
            ("Глава 3. Начало", "Глава 3. Начало"),
            ("第一章 开始", "第一章 开始"),
            ("　　第 12 章　重逢", "第 12 章　重逢"),
            ("【第一百零五回】", "第一百零五回"),
            ("第3卷 风起", "第3卷 风起"),
            ("Lời mở đầu", "Lời mở đầu"),
            ("Prologue: The Fall", "Prologue: The Fall"),
            ("楔子", "楔子"),
            ("番外一 春日", "番外一 春日"),
        ] {
            let want = (!label.is_empty()).then(|| label.to_string());
            assert_eq!(heading(line), want, "{line:?}");
        }
    }

    #[test]
    fn rejects_story_text() {
        for line in [
            "Chương này kể về những ngày mưa.",
            "Chapters were never his thing",
            "Tập trung vào việc chính đi!",
            "Mở đầu bằng một tiếng thét, hắn lao tới",
            "第一次见面的时候，他还很小。",
            "Phần lớn mọi người đều đã về nhà",
            "Book of the dead",
            "Phần một số người đã về",
            "Hồi thứ",
            &format!("Chương 1 {}", "rất dài ".repeat(20)),
        ] {
            assert_eq!(heading(line), None, "{line:?}");
        }
    }

    fn cand(start: u64, len: u64, label: &str) -> Candidate {
        Candidate {
            start,
            end: start + len,
            label: label.into(),
            indented: false,
            ends_sentence: false,
        }
    }

    #[test]
    fn drops_table_of_contents_and_keeps_last_of_a_pair() {
        let mut cands: Vec<_> = (0..5).map(|i| cand(i * 12, 12, "toc")).collect();
        cands.push(cand(5_000, 10, "Quyển 1"));
        cands.push(cand(5_010, 10, "Chương 1"));
        cands.push(cand(9_000, 10, "Chương 2"));
        cands.push(cand(13_000, 10, "Chương 3"));
        let labels: Vec<_> = filter(cands).into_iter().map(|c| c.label).collect();
        assert_eq!(labels, ["Chương 1", "Chương 2", "Chương 3"]);
    }

    #[test]
    fn drops_the_odd_indented_or_sentence_line() {
        let mut cands: Vec<_> = (0..10)
            .map(|i| cand(i * 10_000, 10, &format!("Chương {i}")))
            .collect();
        cands[2].indented = true;
        cands[4].ends_sentence = true;
        let kept = filter(cands);
        assert_eq!(kept.len(), 8);
        assert!(kept.iter().all(|c| !c.indented && !c.ends_sentence));

        // When most headings are indented, indentation means nothing.
        let mut cands: Vec<_> = (0..4).map(|i| cand(i * 10_000, 10, "第一章")).collect();
        for c in &mut cands[1..] {
            c.indented = true;
        }
        assert_eq!(filter(cands).len(), 4);
    }

    #[test]
    fn too_few_headings_means_no_chapters() {
        assert!(filter(vec![cand(0, 10, "a"), cand(9_000, 10, "b")]).is_empty());
    }
}
