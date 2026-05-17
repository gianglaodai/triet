//! Source-level formatter / migrator (v0.7.4.3-error.4a).
//!
//! Currently exposes a single migration rule: `null` → `~0`
//! (ADR-0020 §10 — the `null` keyword is deprecated v0.7.4.3-error,
//! removed at v1.0 as E2002). The rewrite operates at the **lexer**
//! level — we tokenize, find every `Token::Null`, and splice `~0`
//! into the source at those byte spans. Whitespace and comments
//! survive untouched; strings containing the literal text `"null"`
//! are NOT migrated because the lexer reports them as
//! `Token::StringLiteral`, not `Token::Null`.
//!
//! The migration is **idempotent** — re-running it on already-
//! migrated source produces no diff. Authors can safely automate
//! this in a pre-commit hook.

// Bin crates trigger both `unreachable_pub` (cannot reach outside
// the binary) and `clippy::redundant_pub_crate` (pub_crate same as
// pub in a bin). Same trade-off the internal lib crates document.
#![allow(clippy::redundant_pub_crate)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use triet_lexer::{Token, lex};

/// Top-level handler for `triet fmt`. Dispatches to the requested
/// migration rule (currently only `--migrate-null`) and either writes
/// changes back to disk or prints a diff preview to stdout.
pub(crate) fn fmt_command(path: &str, migrate_null: bool, write: bool) -> ExitCode {
    if !migrate_null {
        eprintln!(
            "error: `triet fmt` currently requires at least one rule flag.\n\
             available rules: --migrate-null (ADR-0020 §10: rewrite `null` → `~0`)"
        );
        return ExitCode::from(2);
    }

    let root = Path::new(path);
    let files = match collect_tri_files(root) {
        Ok(list) => list,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(5);
        }
    };

    if files.is_empty() {
        eprintln!("warning: no .tri files found under {}", root.display());
        return ExitCode::SUCCESS;
    }

    let mut changed = 0usize;
    let mut errored = 0usize;
    for file in &files {
        match process_file(file, migrate_null, write) {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(message) => {
                eprintln!("error processing {}: {message}", file.display());
                errored += 1;
            }
        }
    }

    if errored > 0 {
        eprintln!("{errored} file(s) failed to process");
        return ExitCode::from(5);
    }

    if write {
        eprintln!("rewrote {changed} of {} file(s)", files.len());
    } else if changed > 0 {
        eprintln!(
            "dry-run: {changed} of {} file(s) would change. \
             rerun with `--write` to apply.",
            files.len()
        );
    } else {
        eprintln!("{} file(s) checked, no changes needed", files.len());
    }
    ExitCode::SUCCESS
}

/// Walk `root` collecting every `.tri` file path. `root` may itself
/// be a single file (returned as-is when it has the `.tri` extension).
fn collect_tri_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Err(format!("path not found: {}", root.display()));
    }
    let mut acc = Vec::new();
    if root.is_file() {
        if root.extension().and_then(|ext| ext.to_str()) == Some("tri") {
            acc.push(root.to_path_buf());
        } else {
            return Err(format!(
                "file is not a .tri source: {}",
                root.display()
            ));
        }
        return Ok(acc);
    }
    walk_directory(root, &mut acc)
        .map_err(|err| format!("failed to walk {}: {err}", root.display()))?;
    Ok(acc)
}

fn walk_directory(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    // Sort for deterministic traversal — test stability + diff legibility.
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk_directory(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("tri") {
            out.push(path);
        }
    }
    Ok(())
}

/// Process a single file. Returns `Ok(true)` if the file changed,
/// `Ok(false)` if no changes were needed, `Err(_)` on I/O / lex
/// failure.
fn process_file(path: &Path, migrate_null: bool, write: bool) -> Result<bool, String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("cannot read: {err}"))?;
    let rewritten = if migrate_null {
        rewrite_null_to_tilde_zero(&source)?
    } else {
        source.clone()
    };
    if rewritten == source {
        return Ok(false);
    }
    if write {
        fs::write(path, &rewritten).map_err(|err| format!("cannot write: {err}"))?;
        eprintln!("rewrote {}", path.display());
    } else {
        print_diff(path, &source, &rewritten);
    }
    Ok(true)
}

/// Tokenize `source` and replace every `Token::Null` byte span with
/// `~0`. Strings containing the literal `null` (e.g. `"null"`) are
/// lexed as `Token::StringLiteral` so the migration leaves them
/// alone; same for comments which the lexer discards entirely.
fn rewrite_null_to_tilde_zero(source: &str) -> Result<String, String> {
    let tokens = lex(source).map_err(|err| format!("lex failed: {err:?}"))?;
    // Collect spans of every Token::Null. Walking the result back-to-
    // front would also work, but pre-allocating the output and
    // splicing in a single linear pass keeps the function trivial.
    let null_spans: Vec<_> = tokens
        .into_iter()
        .filter_map(|(tok, span)| matches!(tok, Token::Null).then_some(span))
        .collect();
    if null_spans.is_empty() {
        return Ok(source.to_owned());
    }

    let mut output = String::with_capacity(source.len());
    let mut cursor: usize = 0;
    for span in null_spans {
        // The lexer reports spans as byte ranges. The `null` keyword
        // is 4 bytes ASCII; `~0` is 2 bytes. Both endpoints are
        // guaranteed on UTF-8 boundaries since the lexer only emits
        // spans aligned to token boundaries.
        output.push_str(&source[cursor..span.start]);
        output.push_str("~0");
        cursor = span.end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

/// Print a minimal line-oriented diff for dry-run output. We do this
/// by hand rather than pulling in a diff crate — the migration is
/// surgical (4-byte token swap) so a per-line view is enough.
fn print_diff(path: &Path, before: &str, after: &str) {
    println!("--- {}", path.display());
    println!("+++ {} (migrated)", path.display());
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let max_len = before_lines.len().max(after_lines.len());
    for index in 0..max_len {
        let before_line = before_lines.get(index).copied().unwrap_or("");
        let after_line = after_lines.get(index).copied().unwrap_or("");
        if before_line != after_line {
            println!("@@ line {} @@", index + 1);
            if !before_line.is_empty() {
                println!("-{before_line}");
            }
            if !after_line.is_empty() {
                println!("+{after_line}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;

    // ── rewrite_null_to_tilde_zero ──────────────────────────────────

    #[test]
    fn rewrites_bare_null_keyword() {
        let source = "let x = null";
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(result, "let x = ~0");
    }

    #[test]
    fn rewrites_null_in_function_default() {
        let source = "public function lookup(id: Integer) -> User? = null\n";
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(
            result,
            "public function lookup(id: Integer) -> User? = ~0\n"
        );
    }

    #[test]
    fn preserves_null_inside_string_literal() {
        // The lexer reports "null" as a StringLiteral, not Token::Null,
        // so the migration leaves it untouched.
        let source = r#"let s = "null is reserved""#;
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(result, source);
    }

    #[test]
    fn preserves_null_inside_line_comment() {
        // Comments are discarded entirely by the lexer.
        let source = "// uses null as sentinel\nlet x = 0";
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(result, source);
    }

    #[test]
    fn handles_multiple_nulls_in_one_file() {
        let source = "let a = null\nlet b = null\nlet c = null\n";
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(result, "let a = ~0\nlet b = ~0\nlet c = ~0\n");
    }

    #[test]
    fn idempotent_on_already_migrated_source() {
        // Running the migration twice produces no further changes.
        let source = "let x = ~0";
        let pass_one = rewrite_null_to_tilde_zero(source).unwrap();
        let pass_two = rewrite_null_to_tilde_zero(&pass_one).unwrap();
        assert_eq!(pass_one, source);
        assert_eq!(pass_two, source);
    }

    #[test]
    fn empty_input_returns_empty() {
        let result = rewrite_null_to_tilde_zero("").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn no_null_means_no_alloc_required_but_still_returns_owned() {
        // Sanity — file with no `null` returns the source unchanged.
        let source = "function main() -> Integer = 42\n";
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(result, source);
    }

    #[test]
    fn preserves_whitespace_and_formatting_around_null() {
        // The lexer's span ends exactly at the keyword's last byte,
        // so trailing whitespace + newlines stay intact.
        let source = "let x = null   \n    let y = 1";
        let result = rewrite_null_to_tilde_zero(source).unwrap();
        assert_eq!(result, "let x = ~0   \n    let y = 1");
    }

    // ── collect_tri_files ───────────────────────────────────────────

    #[test]
    fn collect_single_tri_file_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.tri");
        fs::write(&file, "function main() -> Integer = 0").unwrap();
        let list = collect_tri_files(&file).unwrap();
        assert_eq!(list, vec![file]);
    }

    #[test]
    fn collect_rejects_non_tri_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "hi").unwrap();
        let result = collect_tri_files(&file);
        assert!(result.is_err());
    }

    #[test]
    fn collect_directory_walks_recursively_filters_tri() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.tri"), "x").unwrap();
        fs::write(root.join("ignored.md"), "y").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("b.tri"), "z").unwrap();
        let mut list = collect_tri_files(root).unwrap();
        list.sort();
        let names: Vec<_> = list
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            vec![PathBuf::from("a.tri"), PathBuf::from("sub").join("b.tri")],
        );
    }

    #[test]
    fn collect_missing_path_errors() {
        let result = collect_tri_files(Path::new("/tmp/__triet_fmt_nonexistent__"));
        assert!(result.is_err());
    }

    // ── process_file integration ────────────────────────────────────

    #[test]
    fn process_file_dry_run_does_not_modify_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.tri");
        fs::write(&file, "let x = null\n").unwrap();
        let changed = process_file(&file, true, false).unwrap();
        assert!(changed);
        let on_disk = fs::read_to_string(&file).unwrap();
        assert_eq!(on_disk, "let x = null\n", "dry-run must not touch disk");
    }

    #[test]
    fn process_file_write_persists_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.tri");
        fs::write(&file, "let x = null\n").unwrap();
        let changed = process_file(&file, true, true).unwrap();
        assert!(changed);
        let on_disk = fs::read_to_string(&file).unwrap();
        assert_eq!(on_disk, "let x = ~0\n");
    }

    #[test]
    fn process_file_no_change_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.tri");
        fs::write(&file, "let x = 0\n").unwrap();
        let changed = process_file(&file, true, true).unwrap();
        assert!(!changed);
    }
}
