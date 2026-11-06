//! Turning whisper output into the text that is inserted.

/// Phrases whisper produces from silence or noise (it learned them from
/// subtitle credits). A result that is only one of these is dropped.
const HALLUCINATIONS: &[&str] = &[
    "textning.nu",
    "svensktextning.nu",
    "www.svensktextning.nu",
    "tack för att du tittade",
    "tack för att ni tittade",
    "tack för att du lyssnade",
    "tack för att ni lyssnade",
    "undertexter från amara.org-gemenskapen",
    "undertexter av amara.org-gemenskapen",
    "thank you for watching",
    "thanks for watching",
    "subtitles by the amara.org community",
    "musik",
    "music",
];

/// Remove `[MUSIK]`, `(skratt)`, `*applåder*` style annotations.
fn strip_annotations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0u32;
    let mut star = false;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            '*' => star = !star,
            _ if depth == 0 && !star => out.push(c),
            _ => {}
        }
    }
    out
}

fn normalise_spaces(text: &str) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for p in [" ,", " .", " ?", " !", " :", " ;"] {
        out = out.replace(p, &p[1..]);
    }
    out
}

fn is_hallucination(text: &str) -> bool {
    let key: String = text
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_lowercase();
    key.is_empty() || HALLUCINATIONS.contains(&key.as_str())
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub capitalize: bool,
    pub trailing_space: bool,
}

/// Clean whisper's output. Returns None when nothing real was said.
pub fn finish(raw: &str, options: Options) -> Option<String> {
    let text = normalise_spaces(&strip_annotations(raw));
    let text = text
        .trim()
        .trim_start_matches(['-', '–', '—'])
        .trim()
        .to_string();
    if is_hallucination(&text) {
        return None;
    }
    let mut text = text;
    if options.capitalize {
        let mut chars = text.chars();
        if let Some(first) = chars.next() {
            text = first.to_uppercase().collect::<String>() + chars.as_str();
        }
    }
    if options.trailing_space && !text.ends_with(char::is_whitespace) {
        text.push(' ');
    }
    Some(text)
}

/// The initial prompt: vocabulary words as a natural sentence fragment, which
/// steers whisper towards these spellings without making it repeat them.
pub fn prompt(words: &[String], language: &str) -> String {
    let words: Vec<&str> = words
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .take(80)
        .collect();
    if words.is_empty() {
        return String::new();
    }
    let lead = match language {
        "sv" => "Ord som förekommer:",
        _ => "Words used:",
    };
    format!("{lead} {}.", words.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Options = Options {
        capitalize: true,
        trailing_space: true,
    };

    #[test]
    fn cleans_and_formats() {
        assert_eq!(
            finish("  hej , jag heter Åsa .", ON).as_deref(),
            Some("Hej, jag heter Åsa. ")
        );
        assert_eq!(
            finish("[MUSIK] öppna niruwm (skratt)", ON).as_deref(),
            Some("Öppna niruwm ")
        );
        let plain = Options {
            capitalize: false,
            trailing_space: false,
        };
        assert_eq!(
            finish(" ärligt talat", plain).as_deref(),
            Some("ärligt talat")
        );
    }

    #[test]
    fn drops_silence_hallucinations() {
        assert_eq!(finish(" Textning.nu", ON), None);
        assert_eq!(finish("Tack för att du tittade!", ON), None);
        assert_eq!(finish("[MUSIK]", ON), None);
        assert_eq!(finish("   ", ON), None);
        assert!(finish("Tack för att du tittade på min presentation", ON).is_some());
    }

    #[test]
    fn prompt_lists_vocabulary() {
        let words = vec!["NIRUX".to_string(), " niruwm ".to_string(), String::new()];
        assert_eq!(prompt(&words, "sv"), "Ord som förekommer: NIRUX, niruwm.");
        assert_eq!(prompt(&[], "en"), "");
    }
}
