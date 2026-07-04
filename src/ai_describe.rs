use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;

use anyhow::{Result, bail};

/// Generate a describe message by running `generate_command` via `sh -c`,
/// piping the change's diff (with an optional bookmark header) to its stdin.
/// Blocking. The change id, bookmark names, and diff byte size are also
/// exported as environment variables for the command to use.
pub fn generate_message(
    diff: &str,
    bookmarks: &[String],
    change_id: &str,
    generate_command: &str,
) -> Result<String> {
    if generate_command.trim().is_empty() {
        bail!(
            "No AI command configured. Set it with:\n  jj config set --user majjit.ai-describe-command '<command>'"
        );
    }
    if diff.trim().is_empty() {
        bail!("No changes to describe");
    }

    let payload = build_stdin_payload(diff, bookmarks);

    let mut child = Command::new("sh")
        .args(["-c", generate_command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("MAJJIT_AI_DESCRIBE_CHANGE_ID", change_id)
        .env("MAJJIT_AI_DESCRIBE_BOOKMARKS", bookmarks.join(" "))
        .env("MAJJIT_AI_DESCRIBE_DIFF_BYTES", diff.len().to_string())
        .spawn()?;

    // Write stdin from a separate thread to avoid a deadlock when the child
    // writes to stdout before consuming all of stdin. Write errors (e.g. the
    // command ignores stdin and exits early → EPIPE) are intentionally ignored.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(payload.as_bytes());
        // Dropping `stdin` here closes the pipe so the child sees EOF.
    });

    let output = child.wait_with_output()?;
    let _ = writer.join();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        bail!("Generate command failed: {}", stderr.trim());
    }

    clean_generated_message(&stdout, &stderr)
}

/// Build the payload fed to the generate command on stdin: the change's diff,
/// optionally preceded by a `Bookmarks:` header when the change has bookmarks.
fn build_stdin_payload(diff: &str, bookmarks: &[String]) -> String {
    if bookmarks.is_empty() {
        diff.to_string()
    } else {
        format!("Bookmarks: {}\n\n{}", bookmarks.join(" "), diff)
    }
}

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
    if let Some(first) = lines.first_mut()
        && let Some(stripped) = first.strip_prefix('`').and_then(|s| s.strip_suffix('`'))
    {
        *first = stripped;
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

    #[test]
    fn test_build_stdin_payload_no_bookmarks() {
        assert_eq!(build_stdin_payload("diff body\n", &[]), "diff body\n");
    }

    #[test]
    fn test_build_stdin_payload_single_bookmark() {
        assert_eq!(
            build_stdin_payload("diff body\n", &["feat/x".to_string()]),
            "Bookmarks: feat/x\n\ndiff body\n"
        );
    }

    #[test]
    fn test_build_stdin_payload_multiple_bookmarks() {
        assert_eq!(
            build_stdin_payload("d\n", &["a".to_string(), "b".to_string()]),
            "Bookmarks: a b\n\nd\n"
        );
    }

    #[test]
    fn test_generate_message_empty_command_errors() {
        let err = generate_message("diff\n", &[], "abc123", "   ").unwrap_err();
        assert!(err.to_string().contains("No AI command configured"));
    }

    #[test]
    fn test_generate_message_empty_diff_errors() {
        let err = generate_message("  \n ", &[], "abc123", "cat").unwrap_err();
        assert_eq!(err.to_string(), "No changes to describe");
    }

    #[test]
    fn test_generate_message_pipes_diff_to_stdin() {
        // `cat` echoes stdin; with no bookmarks the payload is just the diff.
        let out = generate_message("feat: hi\n", &[], "abc123", "cat").unwrap();
        assert_eq!(out, "feat: hi");
    }

    #[test]
    fn test_generate_message_includes_bookmark_header() {
        let out = generate_message("body\n", &["feat/x".to_string()], "abc123", "cat").unwrap();
        assert_eq!(out, "Bookmarks: feat/x\n\nbody");
    }

    #[test]
    fn test_generate_message_exports_bookmark_env() {
        let out = generate_message(
            "d\n",
            &["bk1".to_string(), "bk2".to_string()],
            "chg",
            "printf %s \"$MAJJIT_AI_DESCRIBE_BOOKMARKS\"",
        )
        .unwrap();
        assert_eq!(out, "bk1 bk2");
    }

    #[test]
    fn test_generate_message_nonzero_exit_errors() {
        let err = generate_message("d\n", &[], "chg", "false").unwrap_err();
        assert!(err.to_string().contains("Generate command failed"));
    }
}
