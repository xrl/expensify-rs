//! `expensify skill` — the Claude Code agent skill shipped with this binary.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::{GlobalArgs, SkillCommand, SkillInstallArgs, usage_error};
use crate::commands::note;
use crate::output::View;

/// Embedded from the repository's one copy, so an installed binary can write
/// the skill with no checkout and no network. Because the file the installer
/// writes *is* the file the repository holds, the two cannot drift.
const SKILL: &str = include_str!("../../skill/SKILL.md");

/// Claude Code keys a skill on its directory name, not on the frontmatter.
const SKILL_DIR: &str = "expensify";

/// Dispatch for `expensify skill`.
pub fn run(command: SkillCommand, global: &GlobalArgs) -> Result<()> {
    match command {
        SkillCommand::Install(args) => install(args, global),
    }
}

fn install(args: SkillInstallArgs, global: &GlobalArgs) -> Result<()> {
    if args.print {
        print!("{SKILL}");
        return Ok(());
    }

    let path = skill_path(&skills_root(&args)?);
    if let Some(message) = refusal(&path, args.force) {
        usage_error(message);
    }
    write(&path)?;

    note(
        global,
        "Installed. Claude Code discovers skills at session start, so start a new \
         session to pick it up.",
    );
    let shown = path.display().to_string();
    View::new(
        "skills",
        vec!["PATH"],
        vec![vec![shown.clone()]],
        json!({ "path": shown }),
    )
    .print(global.output)
}

/// Personal directory by default; `--project` is the repository-local
/// convention, and `--skills-dir` names a root outright.
fn skills_root(args: &SkillInstallArgs) -> Result<PathBuf> {
    if let Some(dir) = &args.skills_dir {
        return Ok(dir.clone());
    }
    if args.project {
        return Ok([".claude", "skills"].iter().collect());
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("no home directory in the environment; pass --project or --skills-dir")?;
    Ok(Path::new(&home).join(".claude").join("skills"))
}

fn skill_path(root: &Path) -> PathBuf {
    root.join(SKILL_DIR).join("SKILL.md")
}

/// Why the install must not proceed, if it must not. Split out because
/// reporting it calls `usage_error`, which exits the process — this way the
/// decision itself is testable.
fn refusal(path: &Path, force: bool) -> Option<String> {
    if force || !path.exists() {
        return None;
    }
    Some(format!(
        "{} already exists; pass --force to replace it, or --print to review this \
         version first",
        path.display()
    ))
}

fn write(path: &Path) -> Result<()> {
    let dir = path.parent().expect("skill_path always has a parent");
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(path, SKILL).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    /// Unique per test and per process, so a parallel run cannot collide.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("expensify-skill-{}-{label}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn parse(args: &[&str]) -> SkillInstallArgs {
        match Cli::parse_from(args).command {
            Command::Skill {
                command: SkillCommand::Install(args),
            } => args,
            other => panic!("expected `skill install`, got {other:?}"),
        }
    }

    #[test]
    fn install_defaults_to_the_personal_directory() {
        let args = parse(&["expensify", "skill", "install"]);
        assert!(!args.project && !args.force && !args.print);

        let root = skills_root(&args).unwrap();
        assert!(
            root.ends_with(Path::new(".claude").join("skills")),
            "{root:?}"
        );
        assert!(root.is_absolute(), "{root:?}");
    }

    #[test]
    fn project_and_explicit_roots_are_honoured() {
        let project = parse(&["expensify", "skill", "install", "--project"]);
        assert_eq!(
            skills_root(&project).unwrap(),
            PathBuf::from(".claude").join("skills")
        );

        let explicit = parse(&["expensify", "skill", "install", "--skills-dir", "/tmp/s"]);
        assert_eq!(skills_root(&explicit).unwrap(), PathBuf::from("/tmp/s"));
    }

    /// `--print` writes to stdout; asking it to also install somewhere is a
    /// contradiction clap should catch rather than the installer.
    #[test]
    fn print_refuses_to_be_combined_with_installing() {
        for conflicting in [["--print", "--force"], ["--print", "--project"]] {
            let args = [
                "expensify",
                "skill",
                "install",
                conflicting[0],
                conflicting[1],
            ];
            assert!(Cli::try_parse_from(args).is_err(), "{conflicting:?}");
        }
        assert!(
            Cli::try_parse_from([
                "expensify",
                "skill",
                "install",
                "--project",
                "--skills-dir",
                "/x"
            ])
            .is_err()
        );
    }

    #[test]
    fn installing_writes_the_embedded_skill() {
        let root = TempRoot::new("install");
        let path = skill_path(&root.0);

        assert_eq!(refusal(&path, false), None);
        write(&path).unwrap();

        assert_eq!(path, root.0.join("expensify").join("SKILL.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
    }

    #[test]
    fn a_second_install_is_refused_unless_forced() {
        let root = TempRoot::new("clobber");
        let path = skill_path(&root.0);
        write(&path).unwrap();
        std::fs::write(&path, "edited by hand").unwrap();

        let message = refusal(&path, false).expect("an existing skill must be refused");
        assert!(message.contains("--force"), "{message}");
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited by hand");

        assert_eq!(refusal(&path, true), None);
        write(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
    }

    /// The embedded copy is `include_str!`d from this path, so a mismatch
    /// means the constant was pointed somewhere else — the one way the two
    /// could still drift.
    #[test]
    fn the_embedded_skill_is_the_checked_in_file() {
        let checked_in = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("skill")
            .join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&checked_in).unwrap(), SKILL);
    }

    /// Claude Code reads the frontmatter to decide whether to load the skill;
    /// a malformed header makes the whole file inert.
    #[test]
    fn the_skill_carries_the_frontmatter_claude_code_reads() {
        let mut lines = SKILL.lines();
        assert_eq!(lines.next(), Some("---"));
        let frontmatter: Vec<_> = lines.take_while(|line| *line != "---").collect();
        assert!(frontmatter.contains(&"name: expensify"), "{frontmatter:?}");
        assert!(
            frontmatter
                .iter()
                .any(|line| line.starts_with("description:")),
            "{frontmatter:?}"
        );
    }
}
