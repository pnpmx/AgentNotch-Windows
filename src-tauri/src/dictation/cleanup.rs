//! Light, local transcript cleanup. No text leaves the computer.

/// Hesitation sounds in the supported languages. Only standalone words are
/// removed, never parts of other words.
const FILLERS: [&str; 16] = [
    "um", "uh", "erm", "hmm", "mmm", "eh", "ehm", "em", "este", "ehh", "euh", "äh", "ähm", "ehmm",
    "uhm", "umm",
];

pub fn remove_fillers(text: &str) -> String {
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|word| {
            let bare: String = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            !FILLERS.contains(&bare.as_str())
        })
        .collect();
    let mut out = words.join(" ");
    // Removing a leading filler can leave a stray comma.
    while out.starts_with([',', ';']) {
        out.remove(0);
        out = out.trim_start().to_owned();
    }
    capitalize_first(&out)
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if text != text.to_lowercase() || first.is_lowercase() => {
            first.to_uppercase().collect::<String>() + chars.as_str()
        }
        Some(first) => first.to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_standalone_fillers_only() {
        assert_eq!(
            remove_fillers("Um, fix the, uh, login bug"),
            "Fix the, login bug"
        );
        assert_eq!(remove_fillers("este bug es raro"), "Bug es raro");
        assert_eq!(remove_fillers("the umbrella test"), "The umbrella test");
        assert_eq!(remove_fillers("Ähm, bitte testen"), "Bitte testen");
        assert_eq!(remove_fillers("um"), "");
    }
}
