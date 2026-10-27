//! The framed input and validated answer of a session diagnosis.

pub mod answer;
pub mod input;

/// The bundled skill's `SKILL.md`, as committed.
pub const SKILL_MD: &str = include_str!("../session-diagnosis/SKILL.md");
/// The bundled skill's name.
pub const SKILL_ID: &str = "xtrace-session-diagnosis";

/// The skill's instructions: its `SKILL.md` without the `---` header.
pub fn instructions() -> &'static str {
    strip_frontmatter(SKILL_MD)
}

fn strip_frontmatter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---\n") {
        Some(end) => rest[end + 5..].trim_start_matches('\n'),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_skill_is_named_and_its_instructions_have_no_header() {
        assert!(SKILL_MD.starts_with(&format!("---\nname: {SKILL_ID}\n")));
        let body = instructions();
        assert!(body.starts_with("# XTrace session diagnosis"));
        assert!(!body.contains("\nname: "));
        // The note for the app that builds the input is in Rust, not in the
        // prompt.
        assert!(!body.contains("For the app that builds this input"));
        // The kinds the skill names are the ones the input writes.
        for kind in [
            input::WarningKind::Correction,
            input::WarningKind::Interruption,
            input::WarningKind::RepeatedCalls,
            input::WarningKind::RepeatedPrompt,
            input::WarningKind::RepeatedFailure,
        ] {
            assert!(body.contains(&format!("`{}`", kind.as_str())), "{kind:?}");
        }
    }

    #[test]
    fn the_sample_input_follows_the_input_rules() {
        let sample = include_str!("../session-diagnosis/examples/sample-input.txt");
        assert!(sample.starts_with(&format!("{}\nprotocol: 1\n", input::FIRST_LINE)));
        assert!(sample.contains(&format!("\n{}\n", input::MESSAGES_END)));
        assert!(sample.trim_end().ends_with(input::LAST_LINE));
    }
}
