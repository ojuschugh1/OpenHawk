// hawk setup — auto-install and update all companion tools
//
// Tools managed:
//   sqz        https://github.com/ojuschugh1/sqz        (Rust, install.sh)
//   ghostdep   https://github.com/ojuschugh1/ghostdep   (Rust, install.sh)
//   claimcheck https://github.com/ojuschugh1/claimcheck (Rust, cargo install --force)
//   etch        https://github.com/ojuschugh1/etch       (Go, git clone + make build)
//   aura        https://github.com/ojuschugh1/Aura       (Go, install.sh to ~/.local/bin)
//
// Fix notes:
//   claimcheck — always pass --force to cargo install so it reinstalls even when
//                the binary is already present (avoids "already installed" no-op)
//   etch       — go proxy may be blocked; fall back to git clone + go build
//   aura       — install.sh writes to /usr/local/bin which needs sudo; we set
//                INSTALL_DIR=~/.local/bin so it installs without root

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

// ── Setup marker ──────────────────────────────────────────────────────────────

pub fn setup_marker_path() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hawk")
        .join(".setup_done")
}

pub fn setup_done() -> bool {
    setup_marker_path().exists()
}

pub fn mark_setup_done() {
    let path = setup_marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, env!("CARGO_PKG_VERSION"));
}

// ── Tool definitions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub check_cmd: &'static str,
    pub install: InstallMethod,
    pub repo: &'static str,
}

#[derive(Debug, Clone)]
pub enum InstallMethod {
    /// curl -fsSL <url> | sh  (with optional env vars)
    CurlSh {
        url: &'static str,
        env: &'static [(&'static str, &'static str)],
    },
    /// cargo install --git <url> --force
    CargoGit(&'static str),
    /// go install <pkg>@latest, with git-clone fallback
    GoInstall {
        pkg: &'static str,
        repo: &'static str,
        bin: &'static str,
    },
}

pub fn all_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "sqz",
            description: "LLM token compression — saves 60-92% on repeated file reads",
            check_cmd: "sqz",
            install: InstallMethod::CurlSh {
                url: "https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh",
                env: &[],
            },
            repo: "https://github.com/ojuschugh1/sqz",
        },
        Tool {
            name: "ghostdep",
            description: "Phantom dependency detector — finds unused and missing packages",
            check_cmd: "ghostdep",
            install: InstallMethod::CurlSh {
                url: "https://raw.githubusercontent.com/ojuschugh1/ghostdep/main/install.sh",
                env: &[],
            },
            repo: "https://github.com/ojuschugh1/ghostdep",
        },
        Tool {
            name: "claimcheck",
            description: "Agent claim verifier — checks files, git, lockfiles against agent claims",
            check_cmd: "claimcheck",
            // always --force so it reinstalls even when already present
            install: InstallMethod::CargoGit("https://github.com/ojuschugh1/claimcheck"),
            repo: "https://github.com/ojuschugh1/claimcheck",
        },
        Tool {
            name: "etch",
            description: "API drift detector — records and diffs real API responses",
            check_cmd: "etch",
            install: InstallMethod::GoInstall {
                pkg: "github.com/ojuschugh1/etch/cmd/etch",
                repo: "https://github.com/ojuschugh1/etch",
                bin: "etch",
            },
            repo: "https://github.com/ojuschugh1/etch",
        },
        Tool {
            name: "aura",
            description: "Persistent cross-tool memory, wiki, MCP proxy, OWASP scoring",
            check_cmd: "aura",
            // set INSTALL_DIR to ~/.local/bin to avoid needing sudo for /usr/local/bin
            install: InstallMethod::CurlSh {
                url: "https://raw.githubusercontent.com/ojuschugh1/Aura/main/install.sh",
                env: &[("INSTALL_DIR", "")], // filled at runtime with ~/.local/bin
            },
            repo: "https://github.com/ojuschugh1/aura",
        },
    ]
}

// ── Availability checks ───────────────────────────────────────────────────────

pub fn tool_available(check_cmd: &str) -> bool {
    // try --version first, then version (no dashes), then --help
    for arg in &["--version", "version", "--help"] {
        if Command::new(check_cmd)
            .arg(arg)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

// ── Installation ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum InstallResult {
    AlreadyInstalled,
    Installed,
    Updated,
    Failed(String),
}

pub fn install_tool(tool: &Tool, force_update: bool) -> InstallResult {
    let already = tool_available(tool.check_cmd);

    if already && !force_update {
        return InstallResult::AlreadyInstalled;
    }

    let result = match &tool.install {
        InstallMethod::CurlSh { url, env } => run_curl_sh(url, env, tool.name),
        InstallMethod::CargoGit(repo) => run_cargo_git(repo),
        InstallMethod::GoInstall { pkg, repo, bin } => run_go_install(pkg, repo, bin),
    };

    match result {
        Ok(()) => {
            if already {
                InstallResult::Updated
            } else {
                InstallResult::Installed
            }
        }
        Err(e) => InstallResult::Failed(e),
    }
}

fn local_bin_dir() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("bin")
}

fn run_curl_sh(url: &str, extra_env: &[(&str, &str)], tool_name: &str) -> Result<(), String> {
    // Ensure ~/.local/bin exists (used as fallback install dir)
    let local_bin = local_bin_dir();
    let _ = std::fs::create_dir_all(&local_bin);

    // Build env: set INSTALL_DIR for tools that respect it (aura, sqz, ghostdep)
    // This avoids needing sudo for /usr/local/bin
    let install_dir = local_bin.to_string_lossy().into_owned();

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!("curl -fsSL '{url}' | sh"))
        .env("INSTALL_DIR", &install_dir)
        .env("BIN_DIR", &install_dir)
        .env("PREFIX", &install_dir);

    // apply any tool-specific env overrides
    for (k, v) in extra_env {
        let val = if v.is_empty() {
            install_dir.as_str()
        } else {
            v
        };
        cmd.env(k, val);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to run curl: {e}"))?;

    if status.success() {
        // Add ~/.local/bin to PATH hint if not already there
        let path_env = std::env::var("PATH").unwrap_or_default();
        if !path_env.contains(local_bin.to_str().unwrap_or("")) {
            println!();
            println!(
                "  note: {} was installed to {}",
                tool_name,
                local_bin.display()
            );
            println!(
                "  add to your shell: export PATH=\"{}:$PATH\"",
                local_bin.display()
            );
        }
        Ok(())
    } else {
        Err(format!("install script exited with status {status}"))
    }
}

fn run_cargo_git(repo: &str) -> Result<(), String> {
    if !Command::new("cargo")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Err("cargo not found — install Rust from https://rustup.rs".to_string());
    }

    // Always pass --force so it reinstalls even when the binary is already present.
    // Without --force, cargo install exits 0 but prints "already installed" and
    // does nothing — which we'd misread as success.
    let status = Command::new("cargo")
        .args(["install", "--git", repo, "--force"])
        .status()
        .map_err(|e| format!("cargo install failed: {e}"))?;

    if status.success() {
        return Ok(());
    }

    // retry without --locked in case Cargo.lock is absent
    let status2 = Command::new("cargo")
        .args(["install", "--git", repo, "--force", "--no-locked"])
        .status()
        .map_err(|e| format!("cargo install failed: {e}"))?;

    if status2.success() {
        Ok(())
    } else {
        Err(format!("cargo install exited with status {status2}"))
    }
}

fn run_go_install(pkg: &str, repo: &str, bin: &str) -> Result<(), String> {
    let go_available = Command::new("go")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !go_available {
        return Err("go not found — install Go from https://go.dev/dl/".to_string());
    }

    // Try go install first (requires proxy access)
    let status = Command::new("go")
        .args(["install", &format!("{pkg}@latest")])
        .status()
        .map_err(|e| format!("go install failed: {e}"))?;

    if status.success() {
        return Ok(());
    }

    // Fallback: git clone + go build + copy to ~/.local/bin
    // This works even when the Go proxy is blocked
    eprintln!("  go proxy unreachable, falling back to git clone...");
    run_go_build_from_source(repo, bin)
}

fn run_go_build_from_source(repo: &str, bin: &str) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("hawk-build-{bin}"));
    let _ = std::fs::remove_dir_all(&tmp);

    // git clone
    let status = Command::new("git")
        .args(["clone", "--depth=1", repo, tmp.to_str().unwrap_or(".")])
        .status()
        .map_err(|e| format!("git clone failed: {e}"))?;

    if !status.success() {
        return Err(format!("git clone of {repo} failed"));
    }

    // go build
    let out_bin = tmp.join(bin);
    let status = Command::new("go")
        .args(["build", "-o", out_bin.to_str().unwrap_or(bin), "./cmd/..."])
        .current_dir(&tmp)
        .env("GOFLAGS", "-mod=mod")
        .status()
        .map_err(|e| format!("go build failed: {e}"))?;

    if !status.success() {
        // try without ./cmd/... (some repos have main at root)
        let status2 = Command::new("go")
            .args(["build", "-o", out_bin.to_str().unwrap_or(bin), "."])
            .current_dir(&tmp)
            .env("GOFLAGS", "-mod=mod")
            .status()
            .map_err(|e| format!("go build failed: {e}"))?;
        if !status2.success() {
            return Err(format!("go build failed for {repo}"));
        }
    }

    // copy to ~/.local/bin
    let dest_dir = local_bin_dir();
    let _ = std::fs::create_dir_all(&dest_dir);
    let dest = dest_dir.join(bin);

    std::fs::copy(&out_bin, &dest)
        .map_err(|e| format!("failed to copy binary to {}: {e}", dest.display()))?;

    // make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).map_err(|e| e.to_string())?;
    }

    let _ = std::fs::remove_dir_all(&tmp);

    let path_env = std::env::var("PATH").unwrap_or_default();
    if !path_env.contains(dest_dir.to_str().unwrap_or("")) {
        println!();
        println!("  note: {bin} was installed to {}", dest_dir.display());
        println!(
            "  add to your shell: export PATH=\"{}:$PATH\"",
            dest_dir.display()
        );
    }

    Ok(())
}

// ── Main setup flow ───────────────────────────────────────────────────────────

pub struct SetupOptions {
    #[allow(dead_code)]
    pub skip_installed: bool,
    pub force_update: bool,
    pub yes: bool,
    pub only: Vec<String>,
    pub skip: Vec<String>,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            skip_installed: true,
            force_update: false,
            yes: false,
            only: Vec::new(),
            skip: Vec::new(),
        }
    }
}

pub fn run_setup(opts: SetupOptions) -> anyhow::Result<()> {
    let tools = all_tools();

    let tools: Vec<&Tool> = tools
        .iter()
        .filter(|t| {
            if !opts.only.is_empty() {
                return opts.only.iter().any(|n| n == t.name);
            }
            if opts.skip.iter().any(|n| n == t.name) {
                return false;
            }
            true
        })
        .collect();

    println!("OpenHawk companion tools setup");
    println!("{}", "=".repeat(50));
    println!();

    println!("{:<14} {:<12} DESCRIPTION", "TOOL", "STATUS");
    println!("{}", "-".repeat(70));
    for tool in &tools {
        let status = if tool_available(tool.check_cmd) {
            "installed"
        } else {
            "missing"
        };
        println!("{:<14} {:<12} {}", tool.name, status, tool.description);
    }
    println!();

    let to_install: Vec<&&Tool> = tools
        .iter()
        .filter(|t| opts.force_update || !tool_available(t.check_cmd))
        .collect();

    if to_install.is_empty() {
        println!("All tools are already installed.");
        mark_setup_done();
        return Ok(());
    }

    println!("Will install/update {} tool(s):", to_install.len());
    for t in &to_install {
        println!("  {} — {}", t.name, t.repo);
    }
    println!();

    if !opts.yes {
        print!("Proceed? [Y/n] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_lowercase();
        if trimmed == "n" || trimmed == "no" {
            println!("Setup cancelled.");
            return Ok(());
        }
    }

    println!();

    let mut installed = 0;
    let mut failed = 0;

    for tool in &to_install {
        print!("Installing {}... ", tool.name);
        io::stdout().flush()?;

        let start = Instant::now();
        let result = install_tool(tool, opts.force_update);
        let elapsed = start.elapsed().as_secs();

        match result {
            InstallResult::Installed => {
                println!("done ({elapsed}s)");
                installed += 1;
            }
            InstallResult::Updated => {
                println!("updated ({elapsed}s)");
                installed += 1;
            }
            InstallResult::AlreadyInstalled => {
                println!("already installed");
            }
            InstallResult::Failed(ref e) => {
                println!("FAILED");
                eprintln!("  error: {e}");
                eprintln!("  install manually: {}", tool.repo);
                failed += 1;
            }
        }
    }

    println!();
    println!("Setup complete: {installed} installed, {failed} failed.");

    if failed == 0 {
        mark_setup_done();
        println!();
        println!("All tools ready. Run 'openhawk --help' to get started.");
    } else {
        println!();
        println!("Some tools failed. Retry with:");
        println!("  hawk setup --yes");
        println!();
        println!("Or install manually:");
        for t in &to_install {
            if let InstallResult::Failed(_) = install_tool(t, false) {
                println!("  {} — {}", t.name, t.repo);
            }
        }
    }

    Ok(())
}

pub fn maybe_first_run() {
    if setup_done() {
        return;
    }

    let missing: Vec<&str> = all_tools()
        .iter()
        .filter(|t| !tool_available(t.check_cmd))
        .map(|t| t.name)
        .collect();

    if missing.is_empty() {
        mark_setup_done();
        return;
    }

    println!();
    println!("Welcome to OpenHawk!");
    println!();
    println!("The following companion tools are not installed:");
    for name in &missing {
        println!("  {name}");
    }
    println!();
    println!("Run 'openhawk setup' to install them automatically.");
    println!("Run 'openhawk setup --yes' to install without prompts.");
    println!();

    mark_setup_done();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_returns_five_tools() {
        assert_eq!(all_tools().len(), 5);
    }

    #[test]
    fn tool_names_are_unique() {
        let tools = all_tools();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        names.dedup();
        assert_eq!(names.len(), tools.len());
    }

    #[test]
    fn tool_available_check_does_not_panic() {
        for tool in all_tools() {
            let _ = tool_available(tool.check_cmd);
        }
    }

    #[test]
    fn setup_marker_path_is_under_home() {
        let path = setup_marker_path();
        let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
        assert!(path.starts_with(home));
    }

    #[test]
    fn local_bin_dir_is_under_home() {
        let dir = local_bin_dir();
        let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
        assert!(dir.starts_with(home));
    }

    #[test]
    fn install_result_variants_compile() {
        let r = InstallResult::AlreadyInstalled;
        assert!(matches!(r, InstallResult::AlreadyInstalled));
        let r = InstallResult::Failed("test".to_string());
        assert!(matches!(r, InstallResult::Failed(_)));
    }

    #[test]
    fn setup_options_defaults() {
        let opts = SetupOptions::default();
        assert!(opts.skip_installed);
        assert!(!opts.force_update);
        assert!(!opts.yes);
        assert!(opts.only.is_empty());
        assert!(opts.skip.is_empty());
    }

    #[test]
    fn aura_install_uses_local_bin_env() {
        // verify aura's install method sets INSTALL_DIR
        let tools = all_tools();
        let aura = tools.iter().find(|t| t.name == "aura").unwrap();
        match &aura.install {
            InstallMethod::CurlSh { env, .. } => {
                assert!(env.iter().any(|(k, _)| *k == "INSTALL_DIR"));
            }
            _ => panic!("aura should use CurlSh"),
        }
    }

    #[test]
    fn claimcheck_uses_cargo_git() {
        let tools = all_tools();
        let cc = tools.iter().find(|t| t.name == "claimcheck").unwrap();
        assert!(matches!(cc.install, InstallMethod::CargoGit(_)));
    }

    #[test]
    fn etch_uses_go_install_with_fallback() {
        let tools = all_tools();
        let etch = tools.iter().find(|t| t.name == "etch").unwrap();
        assert!(matches!(etch.install, InstallMethod::GoInstall { .. }));
    }
}
