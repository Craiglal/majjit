use anyhow::{Result, bail};

fn clean_generated_message(stdout: &str, stderr: &str) -> Result<String> {
    let message = strip_markdown_fences(stdout);
    if message.trim().is_empty() {
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("Generate command produced no commit message");
        }
        bail!("Generate command produced no commit message: {}", stderr);
    }
    Ok(message)
}

/// Strip markdown code fences and any preamble text before the message.
fn strip_markdown_fences(raw: &str) -> String {
    let trimmed = raw.trim();

    // If the output contains a code fence, extract content from within it.
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        // Skip the language identifier on the opening fence line.
        let content_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_fence[content_start..];

        if let Some(end) = content.find("```") {
            return content[..end].trim().to_string();
        }
        // No closing fence — use everything after the opening.
        return content.trim().to_string();
    }

    // Strip a pair of single backticks wrapping only the first line.
    let mut lines: Vec<&str> = trimmed.lines().collect();
    if let Some(first) = lines.first_mut() {
        if let Some(stripped) = first.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
            *first = stripped;
        }
    }

    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown_fences_plain() {
        assert_eq!(
            strip_markdown_fences("fix: update login"),
            "fix: update login"
        );
    }

    #[test]
    fn test_strip_markdown_fences_with_fences() {
        let input = "Here's a commit message:\n\n```\nfeat: add user auth\n\nAdded JWT-based authentication.\n```\n";
        assert_eq!(
            strip_markdown_fences(input),
            "feat: add user auth\n\nAdded JWT-based authentication."
        );
    }

    #[test]
    fn test_strip_single_backticks() {
        assert_eq!(
            strip_markdown_fences("`feat: blah blah blah`"),
            "feat: blah blah blah"
        );
    }

    #[test]
    fn test_strip_single_backticks_first_line_only() {
        let input =
            "`feat: something something`\n\nother content of the commit here stuff\nblah blah blah";
        assert_eq!(
            strip_markdown_fences(input),
            "feat: something something\n\nother content of the commit here stuff\nblah blah blah"
        );
    }

    #[test]
    fn test_strip_markdown_fences_with_language() {
        let input = "```text\nfix: resolve race condition\n```";
        assert_eq!(strip_markdown_fences(input), "fix: resolve race condition");
    }

    #[test]
    fn test_clean_generated_message_rejects_empty_stdout() {
        let err = clean_generated_message("\n\t\n", "").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Generate command produced no commit message"
        );
    }

    #[test]
    fn test_clean_generated_message_rejects_empty_fence() {
        let err = clean_generated_message("```text\n\n```", "model returned nothing").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Generate command produced no commit message: model returned nothing"
        );
    }
}
