use crate::log_tree::COMMIT_FIELD_MARKER;
use crate::model::GlobalArgs;
use crate::terminal::{self, Term};
use anyhow::{Result, anyhow};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use std::{
    env,
    io::{Read, Write},
    process::{Command, Stdio},
};

#[derive(Debug)]
pub struct JjCommand {
    args: Vec<String>,
    global_args: GlobalArgs,
    interactive_term: Option<Term>,
    return_output: ReturnOutput,
    pub sync: bool,
    color: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "The Stage 1 adapter is wired by a later model integration."
)]
pub enum TuicrTarget {
    WorkingCopy,
    Change(String),
    Range { base: String, tip: String },
}

#[derive(Debug)]
pub enum TuicrError {
    Launch(anyhow::Error),
    Terminal(anyhow::Error),
}

impl std::fmt::Display for TuicrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Launch(error) | Self::Terminal(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TuicrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Launch(error) | Self::Terminal(error) => error.source(),
        }
    }
}

#[allow(
    dead_code,
    reason = "The Stage 1 adapter is wired by a later model integration."
)]
fn tuicr_args(target: &TuicrTarget) -> Vec<String> {
    match target {
        TuicrTarget::WorkingCopy => vec!["-w".to_string()],
        TuicrTarget::Change(change) => vec!["-r".to_string(), format!("{change}-..{change}")],
        TuicrTarget::Range { base, tip } => vec!["-r".to_string(), format!("{base}..{tip}")],
    }
}

#[allow(
    dead_code,
    reason = "The Stage 1 adapter is wired by a later model integration."
)]
fn tuicr_command(repository: &str, target: &TuicrTarget) -> Command {
    let mut command = Command::new("tuicr");
    command.args(tuicr_args(target)).current_dir(repository);
    command
}

#[allow(
    dead_code,
    reason = "The Stage 1 adapter is wired by a later model integration."
)]
pub fn run_tuicr(
    term: &Term,
    repository: &str,
    target: TuicrTarget,
) -> std::result::Result<(), TuicrError> {
    run_tuicr_with(
        repository,
        &target,
        terminal::relinquish_terminal,
        || terminal::takeover_terminal(term),
        |command| command.status().map(|status| status.success()),
    )
}

#[allow(
    dead_code,
    reason = "The Stage 1 adapter is wired by a later model integration."
)]
fn run_tuicr_with<Relinquish, Takeover, Run>(
    repository: &str,
    target: &TuicrTarget,
    relinquish: Relinquish,
    takeover: Takeover,
    run: Run,
) -> std::result::Result<(), TuicrError>
where
    Relinquish: FnOnce() -> Result<()>,
    Takeover: FnOnce() -> Result<()>,
    Run: FnOnce(&mut Command) -> std::io::Result<bool>,
{
    relinquish().map_err(TuicrError::Terminal)?;
    let mut command = tuicr_command(repository, target);
    let process_result = run(&mut command);
    takeover().map_err(TuicrError::Terminal)?;
    match process_result {
        Ok(true) => Ok(()),
        Ok(false) => Err(TuicrError::Launch(anyhow!("tuicr exited unsuccessfully"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(TuicrError::Launch(anyhow!(
                "Unable to start tuicr: it was not found on PATH. Please install it from https://tuicr.dev."
            )))
        }
        Err(error) => Err(TuicrError::Launch(error.into())),
    }
}

#[derive(Debug)]
enum ReturnOutput {
    Combined,
    Stdout,
    Stderr,
}

#[derive(Debug)]
struct JjCommandOutput {
    stdout: String,
    stderr: String,
}

impl JjCommand {
    fn new(
        args: &[&str],
        global_args: GlobalArgs,
        interactive_term: Option<Term>,
        return_output: ReturnOutput,
    ) -> Self {
        Self {
            args: args.iter().map(|a| a.to_string()).collect(),
            global_args,
            interactive_term,
            return_output,
            sync: true,
            color: true,
        }
    }

    fn new_skip_sync(
        args: &[&str],
        global_args: GlobalArgs,
        interactive_term: Option<Term>,
        return_output: ReturnOutput,
    ) -> Self {
        Self {
            args: args.iter().map(|a| a.to_string()).collect(),
            global_args,
            interactive_term,
            return_output,
            sync: false,
            color: true,
        }
    }

    fn new_no_color(args: &[&str], global_args: GlobalArgs, return_output: ReturnOutput) -> Self {
        Self {
            args: args.iter().map(|a| a.to_string()).collect(),
            global_args,
            interactive_term: None,
            return_output,
            sync: false,
            color: false,
        }
    }

    pub fn to_lines(&self) -> Vec<Line<'static>> {
        let line = Line::from(vec![
            Span::styled("❯", Style::default().fg(Color::Yellow)),
            Span::raw(" jj "),
            Span::raw(self.args.join(" ")),
        ]);
        let blank_line = Line::raw("");
        vec![line, blank_line]
    }

    pub fn run(&self) -> Result<String, JjCommandError> {
        let output = match &self.interactive_term {
            None => self.run_noninteractive(),
            Some(term) => self.run_interactive(term),
        }?;
        match self.return_output {
            ReturnOutput::Combined => Ok(combine_output(output.stdout, output.stderr)),
            ReturnOutput::Stdout => Ok(output.stdout),
            ReturnOutput::Stderr => Ok(output.stderr),
        }
    }

    fn run_noninteractive(&self) -> Result<JjCommandOutput, JjCommandError> {
        let mut command = self.base_command();
        command.args(self.args.clone());
        let output = command.output().map_err(JjCommandError::new_other)?;

        let stderr = String::from_utf8_lossy(&output.stderr).into();
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).into();
            Ok(JjCommandOutput { stdout, stderr })
        } else {
            Err(JjCommandError::new_failed(stderr))
        }
    }

    fn run_interactive(&self, term: &Term) -> Result<JjCommandOutput, JjCommandError> {
        let mut command = self.base_command();
        command.args(self.args.clone());
        command.stderr(std::process::Stdio::piped());

        terminal::relinquish_terminal().map_err(JjCommandError::new_other)?;

        let mut child = command.spawn().map_err(JjCommandError::new_other)?;
        let mut stderr_handle = child
            .stderr
            .take()
            .ok_or_else(|| JjCommandError::new_other(anyhow!("No stderr handle")))?;
        let mut buf = Vec::new();
        stderr_handle
            .read_to_end(&mut buf)
            .map_err(JjCommandError::new_other)?;
        let stderr = strip_non_style_ansi(&String::from_utf8_lossy(&buf));
        let status = child.wait().map_err(JjCommandError::new_other)?;

        terminal::takeover_terminal(term).map_err(JjCommandError::new_other)?;

        if status.success() {
            Ok(JjCommandOutput {
                stdout: "".to_string(),
                stderr,
            })
        } else {
            Err(JjCommandError::new_failed(stderr))
        }
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new("jj");
        let args = [
            "--color",
            if self.color { "always" } else { "never" },
            "--config",
            "ui.pager=:builtin",
            "--config",
            "ui.diff-editor=:builtin",
            "--config",
            "ui.conflict-marker-style=snapshot",
            "--config",
            "ui.streampager.interface=full-screen-clear-output",
            "--config",
            "template-aliases.\"format_short_change_id(id)\"=format_short_id(id)",
            "--config",
            "template-aliases.\"format_short_id(id)\"=id.shortest(8)",
            "--config",
            r#"template-aliases."format_short_signature(signature)"="coalesce(signature.email(), email_placeholder)""#,
            "--config",
            r#"template-aliases."format_timestamp(timestamp)"='timestamp.local().format("%Y-%m-%d %H:%M:%S")'"#,
            "--config",
            r#"templates.log_node=
                coalesce(
                  if(!self, label("elided", "~")),
                  label(
                    separate(" ",
                      if(current_working_copy, "working_copy"),
                      if(immutable, "immutable"),
                      if(conflict, "conflict"),
                    ),
                    coalesce(
                      if(current_working_copy, "@"),
                      if(root, "┴"),
                      if(immutable, "●"),
                      if(conflict, "⊗"),
                      "○",
                    )
                  )
                )
            "#,
            "--repository",
            &self.global_args.repository,
        ];
        command.args(args);

        if self.global_args.ignore_immutable {
            command.arg("--ignore-immutable");
        }

        command
    }

    pub fn jj_log(revset: &str, global_args: GlobalArgs) -> Self {
        let m = COMMIT_FIELD_MARKER;
        let template = format!(
            r#"stringify(concat(
                "{m}", change_id.shortest(8), if(divergent, "/" ++ change_offset),
                "{m}", commit_id.shortest(8),
                "{m}", if(current_working_copy, "Y", "N"),
                "{m}", if(conflict, "Y", "N"),
                "{m}", if(empty, "Y", "N"),
                "{m}", if(root, "Y", "N"),
                "{m}", working_copies,
                "{m}", local_bookmarks.map(|b| b.name()).join(" "),
                "{m}", tags.map(|t| t.name()).join(" "),
                "{m}", coalesce(author.email(), ""),
                "{m}", author.timestamp().local().format("%Y-%m-%d %H:%M:%S"),
                "{m}", coalesce(description.first_line(), ""),
                "{m}"
            )) ++ builtin_log_compact"#,
        );
        let args = ["log", "--template", &template, "--revisions", revset];
        Self::new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn jj_log_targets(revset: &str, global_args: GlobalArgs) -> Self {
        let template = concat!(
            r#"change_id.shortest(8) ++ "\n""#,
            r#" ++ commit_id.shortest(8) ++ "\n""#,
            r#" ++ local_bookmarks.map(|b| b.name()).join("\n") ++ "\n""#,
            r#" ++ remote_bookmarks.filter(|b| b.remote() != "git").map(|b| b.name() ++ "@" ++ b.remote()).join("\n") ++ "\n""#,
            r#" ++ tags.map(|t| t.name()).join("\n") ++ "\n""#,
            r#" ++ working_copies ++ "\n""#,
        );
        let args = vec!["log", "--no-graph", "--revisions", revset, "-T", template];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_diff_summary(change_id: &str, global_args: GlobalArgs) -> Self {
        let args = [
            "diff",
            "--ignore-working-copy",
            "--summary",
            "--revisions",
            change_id,
        ];
        Self::new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn jj_diff_git(change_id: &str, global_args: GlobalArgs) -> Self {
        Self::new_no_color(&diff_git_args(change_id), global_args, ReturnOutput::Stdout)
    }

    pub fn jj_diff_git_file(change_id: &str, file: &str, global_args: GlobalArgs) -> Self {
        Self::new_no_color(
            &diff_git_file_args(change_id, file),
            global_args,
            ReturnOutput::Stdout,
        )
    }

    pub fn jj_diff_git_file_colored(change_id: &str, file: &str, global_args: GlobalArgs) -> Self {
        Self::new(
            &diff_git_file_args(change_id, file),
            global_args,
            None,
            ReturnOutput::Stdout,
        )
    }

    pub fn jj_diff_file_interactive(
        change_id: &str,
        file: &str,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let args = ["diff", "--revisions", change_id, file];
        Self::new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_diff_from_to_interactive(
        from: &str,
        to: &str,
        file: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["diff", "--from", from, "--to", to];
        if let Some(file) = file {
            args.push(file);
        }
        Self::new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_describe_with_message(
        change_id: &str,
        message: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = ["describe", change_id, "-m", message];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    /// Fetch a change's full description text (all lines, not just the first).
    /// Non-interactive; `.run()` returns the raw description — trim before use.
    pub fn jj_description(change_id: &str, global_args: GlobalArgs) -> Self {
        Self::new_no_color(
            &description_args(change_id),
            global_args,
            ReturnOutput::Stdout,
        )
    }

    pub fn jj_duplicate(
        change_id: &str,
        destination_type: Option<&str>,
        destination: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["duplicate", change_id];
        if let (Some(destination_type), Some(destination)) = (destination_type, destination) {
            args.push(destination_type);
            args.push(destination);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_new(target: &str, flags: &[&str], global_args: GlobalArgs) -> Self {
        let mut args = vec!["new"];
        args.extend_from_slice(flags);
        args.push(target);
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_new_merge(
        first_parent: &str,
        second_parent: &str,
        message: &str,
        global_args: GlobalArgs,
    ) -> Self {
        Self::new(
            &new_merge_args(first_parent, second_parent, message),
            global_args,
            None,
            ReturnOutput::Stderr,
        )
    }

    pub fn jj_parallelize(revset: &str, global_args: GlobalArgs) -> Self {
        let args = ["parallelize", revset];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_next_prev(
        direction: &str,
        mode: Option<&str>,
        offset: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec![direction];
        if let Some(mode) = mode {
            args.push(mode);
        }
        if let Some(offset) = offset {
            args.push(offset);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_abandon(change_id: &str, mode: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["abandon"];
        if let Some(mode) = mode {
            args.push(mode);
        }
        args.push(change_id);
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_absorb(
        from_change_id: &str,
        maybe_into_change_id: Option<&str>,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["absorb", "--from", from_change_id];
        if let Some(into_change_id) = maybe_into_change_id {
            args.push("--into");
            args.push(into_change_id);
        }
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_revert(
        revision: &str,
        destination_type: &str,
        destination: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = ["revert", "-r", revision, destination_type, destination];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_sign(action: &str, revset: &str, global_args: GlobalArgs) -> Self {
        let args = [action, "-r", revset];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_show(change_id: &str, global_args: GlobalArgs, term: Term) -> Self {
        let args = ["show", change_id];
        Self::new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_status(global_args: GlobalArgs, term: Term) -> Self {
        let args = ["status"];
        Self::new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_simplify_parents(revision: &str, mode: &str, global_args: GlobalArgs) -> Self {
        let args = ["simplify-parents", mode, revision];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_undo(global_args: GlobalArgs) -> Self {
        let args = ["undo"];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_redo(global_args: GlobalArgs) -> Self {
        let args = ["redo"];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_commit_with_message(
        message: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        Self::new(
            &commit_message_args(message, maybe_file_path),
            global_args,
            None,
            ReturnOutput::Stderr,
        )
    }

    pub fn jj_rebase(
        source_type: &str,
        source: &str,
        destination_type: &str,
        destination: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = vec!["rebase", source_type, source, destination_type, destination];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_raw(args: &str, global_args: GlobalArgs) -> Result<Self> {
        let parsed = shell_words::split(args)?;
        Ok(Self {
            args: parsed,
            global_args,
            interactive_term: None,
            return_output: ReturnOutput::Combined,
            sync: true,
            color: true,
        })
    }

    pub fn jj_raw_interactive(args: &str, global_args: GlobalArgs, term: Term) -> Result<Self> {
        let parsed = shell_words::split(args)?;
        Ok(Self {
            args: parsed,
            global_args,
            interactive_term: Some(term),
            return_output: ReturnOutput::Combined,
            sync: true,
            color: true,
        })
    }

    pub fn jj_split(
        change_id: &str,
        message: &str,
        destination_type: Option<&str>,
        destination: Option<&str>,
        parallel: bool,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        Self::new(
            &split_args(change_id, message, parallel, destination_type, destination),
            global_args,
            Some(term),
            ReturnOutput::Stderr,
        )
    }

    pub fn jj_resolve(
        change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        // meld auto-merges the clean hunks and presents only real conflicts for editing.
        let mut args = vec!["resolve", "--tool", MERGE_TOOL, "-r", change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_restore(
        flags: &[&str],
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["restore"];
        args.extend_from_slice(flags);
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_squash_noninteractive(
        change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["squash", "--revision", change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_squash_interactive(
        change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["squash", "--revision", change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_squash_into_interactive(
        from_change_id: &str,
        into_change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["squash", "--from", from_change_id, "--into", into_change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_edit(change_id: &str, global_args: GlobalArgs) -> Self {
        let args = ["edit", change_id];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_evolog(change_id: &str, patch: bool, global_args: GlobalArgs, term: Term) -> Self {
        let mut args = vec!["evolog", "-r", change_id];
        if patch {
            args.push("--patch");
        }
        Self::new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_interdiff(
        from: &str,
        to: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["interdiff", "--from", from, "--to", to];
        if let Some(path) = maybe_file_path {
            args.push(path);
        }
        Self::new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn jj_file_list(global_args: GlobalArgs) -> Self {
        let args = ["file", "list"];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_file_show(change_id: &str, file_path: &str, global_args: GlobalArgs) -> Self {
        let args = ["file", "show", "--revision", change_id, file_path];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_file_track(file_path: &str, global_args: GlobalArgs) -> Self {
        let args = ["file", "track", file_path];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_file_untrack(file_path: &str, global_args: GlobalArgs) -> Self {
        let args = ["file", "untrack", file_path];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_metaedit(
        change_id: &str,
        flag: &str,
        value: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["metaedit", flag];
        if let Some(value) = value {
            args.push(value);
        }
        args.push(change_id);
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_git_fetch(flag: Option<&str>, value: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["git", "fetch"];
        if let Some(flag) = flag {
            args.push(flag);
        }
        if let Some(value) = value {
            args.push(value);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_git_push(flag: Option<&str>, value: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["git", "push"];
        if let Some(flag) = flag {
            args.push(flag);
        }
        if let Some(value) = value {
            args.push(value);
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_git_remote_list(global_args: GlobalArgs) -> Self {
        let args = ["git", "remote", "list"];
        Self::new_skip_sync(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_create(
        bookmark_names: &str,
        change_id: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = [
            "bookmark",
            "create",
            "--revision",
            change_id,
            bookmark_names,
        ];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_delete(bookmark_names: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "delete", bookmark_names];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_forget(
        bookmark_names: &str,
        include_remotes: bool,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["bookmark", "forget"];
        if include_remotes {
            args.push("--include-remotes");
        }
        args.push(bookmark_names);
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_advance(to_change_id: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "advance", "--to", to_change_id];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_list_all_names(global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "list", "--all-remotes", "-T", r#"name ++ "\n""#];
        Self::new_skip_sync(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_list_tracked_remote(global_args: GlobalArgs) -> Self {
        let args = [
            "bookmark",
            "list",
            "--tracked",
            "-T",
            r#"if(remote, name ++ "@" ++ remote ++ "\n")"#,
        ];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_list_untracked_remote(global_args: GlobalArgs) -> Self {
        let args = [
            "bookmark",
            "list",
            "--all-remotes",
            "-T",
            r#"if(remote && !tracked, name ++ "@" ++ remote ++ "\n")"#,
        ];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_list_all_display(global_args: GlobalArgs) -> Self {
        let args = [
            "bookmark",
            "list",
            "--all-remotes",
            "-T",
            r#"if(remote, name ++ "@" ++ remote, name) ++ "\n""#,
        ];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_list_local_only_names(global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "list", "-T", r#"if(!remote, name ++ "\n")"#];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_list_conflicted_names(global_args: GlobalArgs) -> Self {
        let args = [
            "bookmark",
            "list",
            "--conflicted",
            "-T",
            r#"if(!remote, name ++ "\n")"#,
        ];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_bookmark_move(
        from_change_id: &str,
        to_change_id: &str,
        allow_backwards: bool,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec![
            "bookmark",
            "move",
            "--from",
            from_change_id,
            "--to",
            to_change_id,
        ];
        if allow_backwards {
            args.push("--allow-backwards");
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_rename(
        old_bookmark_name: &str,
        new_bookmark_name: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = ["bookmark", "rename", old_bookmark_name, new_bookmark_name];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_set(
        bookmark_names: &str,
        change_id: &str,
        allow_backwards: bool,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["bookmark", "set", bookmark_names, "--revision", change_id];
        if allow_backwards {
            args.push("--allow-backwards");
        }
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_track(bookmark_at_remote: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "track", bookmark_at_remote];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_bookmark_untrack(bookmark_at_remote: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "untrack", bookmark_at_remote];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_config_get_revsets_log(repository: &str) -> Result<String, JjCommandError> {
        let args = ["--repository", repository, "config", "get", "revsets.log"];
        let output = Command::new("jj")
            .args(args)
            .output()
            .map_err(JjCommandError::new_other)?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout)
                .to_string()
                .trim()
                .to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).into();
            Err(JjCommandError::new_failed(stderr))
        }
    }

    pub fn jj_config_get_ai_describe_command(
        repository: &str,
    ) -> Result<Option<String>, JjCommandError> {
        let args = [
            "--repository",
            repository,
            "config",
            "get",
            "majjit.ai-describe-command",
        ];
        let output = Command::new("jj")
            .args(args)
            .output()
            .map_err(JjCommandError::new_other)?;

        // `jj config get` exits non-zero when the key is unset — treat that as
        // "no command configured" rather than an error to surface. Any other
        // non-zero exit (e.g. a malformed jj config) is intentionally reported
        // the same way: the AI-describe flow is optional, and a genuinely broken
        // jj config surfaces through the many other jj commands majjit runs.
        if !output.status.success() {
            return Ok(None);
        }

        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    pub fn jj_workspace_list_names(global_args: GlobalArgs) -> Self {
        let args = [
            "workspace",
            "list",
            "--ignore-working-copy",
            "-T",
            r#"name ++ "\n""#,
        ];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_workspace_list_current_name(global_args: GlobalArgs) -> Self {
        let args = [
            "workspace",
            "list",
            "--ignore-working-copy",
            "-T",
            r#"if(target.current_working_copy(), name ++ "\n")"#,
        ];
        Self::new_no_color(&args, global_args, ReturnOutput::Stdout)
    }

    pub fn jj_workspace_add(
        destination: &str,
        name: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["workspace", "add"];
        if let Some(n) = name {
            args.push("--name");
            args.push(n);
        }
        args.push(destination);
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_workspace_forget(names: &[&str], global_args: GlobalArgs) -> Self {
        let mut args = vec!["workspace", "forget"];
        args.extend(names);
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_workspace_rename(new_name: &str, global_args: GlobalArgs) -> Self {
        let args = ["workspace", "rename", new_name];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_workspace_update_stale(global_args: GlobalArgs) -> Self {
        let args = ["workspace", "update-stale"];
        Self::new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn jj_ensure_valid_repo(repository: &str) -> Result<String, JjCommandError> {
        let args = [
            "--repository",
            repository,
            "workspace",
            "root",
            "--color",
            "always",
        ];
        let output = Command::new("jj")
            .args(args)
            .output()
            .map_err(JjCommandError::new_other)?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout)
                .to_string()
                .trim()
                .to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).into();
            Err(JjCommandError::new_failed(stderr))
        }
    }
}

fn diff_git_args(change_id: &str) -> [&str; 5] {
    [
        "diff",
        "--ignore-working-copy",
        "--git",
        "--revisions",
        change_id,
    ]
}

fn new_merge_args<'a>(first: &'a str, second: &'a str, message: &'a str) -> [&'a str; 5] {
    ["new", "--message", message, first, second]
}

fn commit_message_args<'a>(message: &'a str, maybe_file_path: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec!["commit", "--message", message];
    if let Some(file_path) = maybe_file_path {
        args.push(file_path);
    }
    args
}

fn split_args<'a>(
    change_id: &'a str,
    message: &'a str,
    parallel: bool,
    destination_type: Option<&'a str>,
    destination: Option<&'a str>,
) -> Vec<&'a str> {
    let mut args = vec!["split", "--revision", change_id, "--message", message];
    if parallel {
        args.push("--parallel");
    }
    if let (Some(destination_type), Some(destination)) = (destination_type, destination) {
        args.push(destination_type);
        args.push(destination);
    }
    args
}

fn description_args(change_id: &str) -> [&str; 7] {
    [
        "log",
        "--ignore-working-copy",
        "--no-graph",
        "--revisions",
        change_id,
        "-T",
        "description",
    ]
}

fn diff_git_file_args<'a>(change_id: &'a str, file: &'a str) -> [&'a str; 6] {
    [
        "diff",
        "--ignore-working-copy",
        "--git",
        "--revisions",
        change_id,
        file,
    ]
}

/// Syntax-highlighting theme handed to delta. A dark theme, to match a dark terminal.
const DELTA_SYNTAX_THEME: &str = "Dracula";

/// 3-way merge tool used by `jj resolve`. meld's built-in jj config auto-merges the
/// clean hunks (`--auto-merge`) and opens a visual editor for the real conflicts.
const MERGE_TOOL: &str = "meld";

/// Render a single file's diff to ANSI-colored text for the log tree.
///
/// When the `delta` pager is on `PATH`, a raw git-format diff is piped through it
/// for richer syntax highlighting. Otherwise we fall back to jj's own colored git
/// diff so the view keeps working without delta installed.
pub fn rendered_file_diff(change_id: &str, file: &str, global_args: GlobalArgs) -> Result<String> {
    if delta_available() {
        let raw = JjCommand::jj_diff_git_file(change_id, file, global_args.clone()).run()?;
        // If delta fails at runtime (an input it rejects, a flag/version mismatch),
        // degrade to jj's own colored git diff rather than breaking the diff view.
        if let Ok(rendered) = delta_render(&raw) {
            return Ok(rendered);
        }
    }
    Ok(JjCommand::jj_diff_git_file_colored(change_id, file, global_args).run()?)
}

/// Whether the `delta` binary is available on `PATH` (probed once, then cached).
fn delta_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("delta")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Pipe a raw git-format diff through `delta` and return its ANSI-colored output.
///
/// `--color-only` leaves the diff structurally untouched (headers and +/- markers
/// intact), so the output still maps line-for-line back to the input; `--no-gitconfig`
/// keeps rendering independent of the user's personal delta configuration.
fn delta_render(raw: &str) -> Result<String> {
    let mut child = Command::new("delta")
        .args([
            "--color-only",
            "--line-numbers",
            "--no-gitconfig",
            "--paging=never",
            "--true-color=always",
            "--width=variable",
            "--syntax-theme",
            DELTA_SYNTAX_THEME,
        ])
        // `--no-gitconfig` ignores git config, but delta still reads these env
        // vars; clear them so rendering stays deterministic and single-column.
        .env_remove("DELTA_FEATURES")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("BAT_THEME")
        .env_remove("BAT_STYLE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    // Feed stdin from a scoped thread so a large diff can't deadlock against a
    // full stdout pipe while we're still writing; the thread borrows `raw`.
    let mut stdin = child.stdin.take().expect("delta stdin was piped");
    let output = std::thread::scope(|s| {
        s.spawn(move || {
            let _ = stdin.write_all(raw.as_bytes());
            // stdin dropped here -> EOF for delta
        });
        child.wait_with_output()
    })?;

    if !output.status.success() {
        return Err(anyhow!("delta exited with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Debug)]
pub enum JjCommandError {
    Failed { stderr: String },
    Other { err: anyhow::Error },
}

impl JjCommandError {
    fn new_failed(stderr: String) -> Self {
        Self::Failed {
            stderr: stderr.trim().to_string(),
        }
    }

    fn new_other(err: impl Into<anyhow::Error>) -> Self {
        Self::Other { err: err.into() }
    }
}

impl std::fmt::Display for JjCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed { stderr } => {
                write!(f, "{stderr}")
            }
            Self::Other { err } => err.fmt(f),
        }
    }
}

impl std::error::Error for JjCommandError {}

fn combine_output(stdout: String, stderr: String) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

pub fn open_file_in_editor(interactive_term: Term, file_path: &str) -> Result<()> {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    terminal::relinquish_terminal()?;
    let status = Command::new(&editor).arg(file_path).status()?;
    terminal::takeover_terminal(&interactive_term)?;
    if !status.success() {
        anyhow::bail!("'{editor}' exited with status {status} for '{file_path}'");
    }
    Ok(())
}

fn strip_non_style_ansi(str: &str) -> String {
    let non_style_ansi_regex =
        Regex::new(r"\x1b(\[[0-9;?]*[ -/]*([@-l]|[n-~])|\].*?(\x07|\x1b\\)|P.*?\x1b\\)").unwrap();
    non_style_ansi_regex.replace_all(str, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuicr_target_arguments_are_exact() {
        assert_eq!(tuicr_args(&TuicrTarget::WorkingCopy), vec!["-w"]);
        assert_eq!(
            tuicr_args(&TuicrTarget::Change("CHANGE".to_string())),
            vec!["-r", "CHANGE-..CHANGE"]
        );
        assert_eq!(
            tuicr_args(&TuicrTarget::Range {
                base: "BASE".to_string(),
                tip: "TIP".to_string(),
            }),
            vec!["-r", "BASE..TIP"]
        );
    }

    #[test]
    fn tuicr_uses_repository_as_current_dir() {
        let command = tuicr_command("/repo", &TuicrTarget::WorkingCopy);

        assert_eq!(command.get_program(), "tuicr");
        assert_eq!(
            command.get_current_dir(),
            Some(std::path::Path::new("/repo"))
        );
    }

    #[test]
    fn tuicr_accepts_successful_process_and_recovers_terminal() {
        let events = std::cell::RefCell::new(Vec::new());

        let result = run_tuicr_with(
            "/repo",
            &TuicrTarget::WorkingCopy,
            || {
                events.borrow_mut().push("relinquish");
                Ok(())
            },
            || {
                events.borrow_mut().push("takeover");
                Ok(())
            },
            |_| {
                events.borrow_mut().push("run");
                Ok(true)
            },
        );

        assert!(result.is_ok());
        assert_eq!(events.into_inner(), ["relinquish", "run", "takeover"]);
    }

    #[test]
    fn tuicr_reclaims_terminal_after_nonzero_exit() {
        let events = std::cell::RefCell::new(Vec::new());

        let error = run_tuicr_with(
            "/repo",
            &TuicrTarget::WorkingCopy,
            || {
                events.borrow_mut().push("relinquish");
                Ok(())
            },
            || {
                events.borrow_mut().push("takeover");
                Ok(())
            },
            |_| {
                events.borrow_mut().push("run");
                Ok(false)
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("tuicr exited unsuccessfully"));
        assert_eq!(events.into_inner(), ["relinquish", "run", "takeover"]);
    }

    #[test]
    fn tuicr_takeover_failure_is_terminal() {
        let events = std::cell::RefCell::new(Vec::new());

        let error = run_tuicr_with(
            "/repo",
            &TuicrTarget::WorkingCopy,
            || {
                events.borrow_mut().push("relinquish");
                Ok(())
            },
            || {
                events.borrow_mut().push("takeover");
                Err(anyhow!("takeover failed"))
            },
            |_| {
                events.borrow_mut().push("run");
                Ok(false)
            },
        )
        .unwrap_err();

        assert!(matches!(error, TuicrError::Terminal(_)));
        assert_eq!(events.into_inner(), ["relinquish", "run", "takeover"]);
    }

    #[test]
    fn tuicr_reclaims_terminal_and_explains_missing_executable() {
        let events = std::cell::RefCell::new(Vec::new());

        let error = run_tuicr_with(
            "/repo",
            &TuicrTarget::WorkingCopy,
            || {
                events.borrow_mut().push("relinquish");
                Ok(())
            },
            || {
                events.borrow_mut().push("takeover");
                Ok(())
            },
            |_| {
                events.borrow_mut().push("run");
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "test missing executable",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(events.into_inner(), ["relinquish", "run", "takeover"]);
        let message = error.to_string();
        assert!(message.contains("tuicr"));
        assert!(message.contains("PATH"));
        assert!(message.contains("https://tuicr.dev"));
    }

    #[test]
    fn tuicr_exposes_a_narrow_terminal_backed_adapter() {
        let _: fn(&Term, &str, TuicrTarget) -> std::result::Result<(), TuicrError> = run_tuicr;
    }

    #[test]
    fn test_diff_git_args() {
        assert_eq!(
            diff_git_args("abc123"),
            [
                "diff",
                "--ignore-working-copy",
                "--git",
                "--revisions",
                "abc123"
            ]
        );
    }

    #[test]
    fn test_description_args() {
        assert_eq!(
            description_args("abc123"),
            [
                "log",
                "--ignore-working-copy",
                "--no-graph",
                "--revisions",
                "abc123",
                "-T",
                "description",
            ]
        );
    }

    #[test]
    fn test_new_merge_args() {
        assert_eq!(
            new_merge_args(
                "main@origin",
                "feat/variables",
                "Merge feat/variables into main"
            ),
            [
                "new",
                "--message",
                "Merge feat/variables into main",
                "main@origin",
                "feat/variables",
            ]
        );
    }

    #[test]
    fn test_commit_message_args_without_path() {
        assert_eq!(
            commit_message_args("feat: add thing", None),
            vec!["commit", "--message", "feat: add thing"]
        );
    }

    #[test]
    fn test_commit_message_args_with_path() {
        assert_eq!(
            commit_message_args("feat: add thing", Some("src/main.rs")),
            vec!["commit", "--message", "feat: add thing", "src/main.rs"]
        );
    }

    #[test]
    fn test_split_args_default() {
        assert_eq!(
            split_args("abc123", "feat: part one", false, None, None),
            vec![
                "split",
                "--revision",
                "abc123",
                "--message",
                "feat: part one"
            ]
        );
    }

    #[test]
    fn test_split_args_parallel() {
        assert_eq!(
            split_args("abc123", "feat: part one", true, None, None),
            vec![
                "split",
                "--revision",
                "abc123",
                "--message",
                "feat: part one",
                "--parallel",
            ]
        );
    }

    #[test]
    fn test_split_args_onto_destination() {
        assert_eq!(
            split_args(
                "abc123",
                "feat: part one",
                false,
                Some("--onto"),
                Some("def456")
            ),
            vec![
                "split",
                "--revision",
                "abc123",
                "--message",
                "feat: part one",
                "--onto",
                "def456",
            ]
        );
    }

    #[test]
    fn test_split_args_parallel_with_destination() {
        assert_eq!(
            split_args("abc123", "m", true, Some("--insert-after"), Some("def456")),
            vec![
                "split",
                "--revision",
                "abc123",
                "--message",
                "m",
                "--parallel",
                "--insert-after",
                "def456",
            ]
        );
    }
}
