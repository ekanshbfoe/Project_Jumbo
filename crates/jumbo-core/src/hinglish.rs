//! The script's writing rule, checked again where a script is approved: narration is Devanagari, or Mixed Hinglish
//! (Devanagari with English words in Latin letters). Hindi written in Latin letters is refused, since Drama 3 reads it
//! with a foreign accent. packages/schema/src/hinglish.ts and worker/jumbo_worker/contracts/rules.py hold the same
//! rule; packages/schema/fixtures/hinglish-cases.json is shared by all three tests.

/// Hindi words that, written in Latin letters, mark a line as Romanized Hindi rather than English.
const ROMANIZED_HINDI: &[&str] = &[
    "hai", "hain", "tha", "thi", "thay", "ka", "ki", "ke", "ko", "ne", "se", "mein", "aur", "nahi", "nahin", "kya",
    "kyun", "kyon", "toh", "bhi", "yeh", "ye", "woh", "wo", "ek", "lekin", "sirf", "abhi", "kuch", "raha", "rahi",
    "rahe", "hoga", "hogi", "gaya", "gayi", "uska", "uski", "uske", "apna", "apni", "mera", "meri", "tera", "teri",
];

fn is_devanagari(c: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&c)
}

/// The text without bracketed cues such as `[laughs]`, which are directions to the voice, not words of the line.
fn without_cues(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => {
                depth -= 1;
                out.push(' ');
            }
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Why `text` breaks the writing rule, or `None` when it is Devanagari or Mixed Hinglish.
pub fn narration_problem(text: &str) -> Option<String> {
    let text = without_cues(text);
    let words: Vec<&str> =
        text.split_whitespace().filter(|w| w.chars().any(|c| is_devanagari(c) || c.is_ascii_alphabetic())).collect();
    let devanagari = words.iter().filter(|w| w.chars().any(is_devanagari)).count();
    let mut romanized: Vec<String> = Vec::new();
    for w in words.iter().filter(|w| !w.chars().any(is_devanagari)) {
        let bare: String = w.chars().filter(char::is_ascii_alphabetic).collect::<String>().to_ascii_lowercase();
        if ROMANIZED_HINDI.contains(&bare.as_str()) && !romanized.contains(&bare) {
            romanized.push(bare);
        }
    }
    if !romanized.is_empty() {
        return Some(format!(
            "narration has Hindi in Latin letters ({}); write it in Devanagari",
            romanized.join(", ")
        ));
    }
    if devanagari == 0 || devanagari * 2 < words.len() {
        return Some(
            "narration must be Devanagari or Mixed Hinglish: at least half its words in Devanagari, English words may stay in Latin"
                .to_owned(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../packages/schema/fixtures/hinglish-cases.json"))
                .expect("fixture");
        let list = |key: &str| {
            cases[key].as_array().expect(key).iter().map(|v| v.as_str().expect("text").to_owned()).collect::<Vec<_>>()
        };
        for text in list("ok") {
            assert_eq!(narration_problem(&text), None, "{text}");
        }
        for text in list("romanized") {
            assert!(narration_problem(&text).expect("refused").contains("Latin letters"), "{text}");
        }
        for text in list("too_little_devanagari") {
            assert!(narration_problem(&text).expect("refused").contains("at least half"), "{text}");
        }
        assert!(narration_problem(&list("romanized")[0]).expect("refused").contains("(ki, thi, lekin, kuch, tha)"));
    }
}
