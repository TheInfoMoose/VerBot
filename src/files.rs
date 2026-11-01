use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Maximum default number of files collected during a CWD scan so large monorepos remain instantaneous.
pub const DEFAULT_MAX_SCAN_FILES: usize = 5000;

/// Metadata for a discovered file in the user's current working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// Forward-slash normalized path relative to the scanned working directory (e.g. `"src/theme.rs"`).
    pub relative_path: String,
    /// Absolute or joined path on disk.
    pub full_path: PathBuf,
    /// True if the file has a `.md` or `.markdown` extension.
    pub is_markdown: bool,
    /// True if the file looks like an implementation plan / roadmap `.md` file
    /// (e.g. `STEPPED_IMPLEMENTATION_PLAN.md`, `ARGON_BOOT_AND_ISO_PLAN.md`, `PLAN.md`).
    pub is_plan_candidate: bool,
}

/// Snapshot of scanned files in a working directory, respecting `.gitignore`.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceFileIndex {
    pub root_dir: PathBuf,
    pub files: Vec<ScannedFile>,
}

impl WorkspaceFileIndex {
    /// Scans `cwd` using `ignore::WalkBuilder`, respecting `.gitignore`, `.ignore`, and global gitignore rules,
    /// while skipping `.git/` and `target/` directories.
    pub fn scan(cwd: &Path) -> Result<Self> {
        Self::scan_with_limit(cwd, DEFAULT_MAX_SCAN_FILES)
    }

    /// Scans `cwd` up to `max_files` entries.
    pub fn scan_with_limit(cwd: &Path, max_files: usize) -> Result<Self> {
        let mut files = Vec::new();

        let walker = WalkBuilder::new(cwd)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true)
            .parents(true)
            .max_depth(Some(8))
            .filter_entry(|entry| {
                if entry.depth() == 0 {
                    return true;
                }
                let name = entry.file_name().to_string_lossy();
                !matches!(
                    name.as_ref(),
                    ".git" | "target" | "node_modules" | ".verbot"
                )
            })
            .build();

        for result in walker {
            let Ok(entry) = result else {
                continue;
            };
            let Some(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_file() {
                continue;
            }

            let full_path = entry.path().to_path_buf();
            let rel_os = full_path.strip_prefix(cwd).unwrap_or(&full_path);
            let relative_path = normalize_rel_path(rel_os);
            if relative_path.is_empty() {
                continue;
            }

            let is_markdown = has_markdown_ext(&relative_path);
            let is_plan_candidate = is_markdown && looks_like_plan_filename(&relative_path);

            files.push(ScannedFile {
                relative_path,
                full_path,
                is_markdown,
                is_plan_candidate,
            });

            if files.len() >= max_files {
                break;
            }
        }

        // Sort deterministically: plan candidates first, then shallowest paths, then lexicographically
        files.sort_by(|a, b| {
            let depth_a = a.relative_path.matches('/').count();
            let depth_b = b.relative_path.matches('/').count();
            depth_a
                .cmp(&depth_b)
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });

        Ok(Self {
            root_dir: cwd.to_path_buf(),
            files,
        })
    }

    /// Returns all `.md` files that look like stepped implementation or architectural plan files,
    /// ordered by relevance (top-level `*PLAN*.md` first, followed by other `.md` files).
    pub fn plan_markdown_candidates(&self) -> Vec<&ScannedFile> {
        let mut candidates: Vec<&ScannedFile> =
            self.files.iter().filter(|f| f.is_markdown).collect();
        candidates.sort_by(|a, b| {
            b.is_plan_candidate
                .cmp(&a.is_plan_candidate)
                .then_with(|| {
                    let depth_a = a.relative_path.matches('/').count();
                    let depth_b = b.relative_path.matches('/').count();
                    depth_a.cmp(&depth_b)
                })
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        candidates
    }

    /// Returns the best default plan `.md` path in the current working directory if one exists
    /// (e.g., `"STEPPED_IMPLEMENTATION_PLAN.md"`).
    pub fn detect_primary_plan_file(&self) -> Option<&ScannedFile> {
        self.files
            .iter()
            .find(|f| f.is_plan_candidate && !f.relative_path.contains('/'))
            .or_else(|| self.files.iter().find(|f| f.is_plan_candidate))
    }

    /// Performs fast fuzzy filtering over discovered relative paths for `@` / `Ctrl+F` path completion.
    ///
    /// If `markdown_only` is true, restricts matches to `.md` files (with plan candidates boosted to the top).
    pub fn fuzzy_search(&self, query: &str, markdown_only: bool, limit: usize) -> Vec<&ScannedFile> {
        let q = query.trim().to_ascii_lowercase();
        let mut scored: Vec<(i64, &ScannedFile)> = Vec::new();

        for file in &self.files {
            if markdown_only && !file.is_markdown {
                continue;
            }
            if let Some(mut score) = fuzzy_score(&file.relative_path, &q) {
                if markdown_only && file.is_plan_candidate {
                    score += 250;
                }
                scored.push((score, file));
            }
        }

        scored.sort_by(|(sa, fa), (sb, fb)| {
            sb.cmp(sa)
                .then_with(|| fa.relative_path.len().cmp(&fb.relative_path.len()))
                .then_with(|| fa.relative_path.cmp(&fb.relative_path))
        });

        scored
            .into_iter()
            .take(limit)
            .map(|(_, f)| f)
            .collect()
    }
}

fn normalize_rel_path(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(os) => Some(os.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn has_markdown_ext(rel_path: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

fn looks_like_plan_filename(rel_path: &str) -> bool {
    let filename = rel_path
        .rsplit('/')
        .next()
        .unwrap_or(rel_path)
        .to_ascii_lowercase();
    filename.contains("plan")
        || filename.contains("roadmap")
        || filename.contains("stepped")
        || filename.contains("todo")
}

/// Computes a fuzzy match score for `candidate` against lowercase `query`.
/// Higher is better; returns `None` if `query` characters do not appear in order in `candidate`.
pub fn fuzzy_score(candidate: &str, query_lower: &str) -> Option<i64> {
    if query_lower.is_empty() {
        // Prefer shallower and shorter paths when query is empty
        let depth = candidate.matches('/').count() as i64;
        return Some(1000 - depth * 50 - candidate.len() as i64);
    }

    let cand_lower = candidate.to_ascii_lowercase();
    let filename_lower = cand_lower.rsplit('/').next().unwrap_or(&cand_lower);

    // Exact filename or prefix match gets highest priority
    if filename_lower == query_lower {
        return Some(5000);
    }
    if filename_lower.starts_with(query_lower) {
        return Some(4000 - filename_lower.len() as i64);
    }
    if let Some(pos) = filename_lower.find(query_lower) {
        return Some(3000 - (pos as i64 * 10) - candidate.len() as i64);
    }
    if let Some(pos) = cand_lower.find(query_lower) {
        return Some(2000 - (pos as i64 * 5) - candidate.len() as i64);
    }

    // Subsequence character match
    let mut score: i64 = 500;
    let mut last_match_idx: Option<usize> = None;
    let cand_chars: Vec<char> = cand_lower.chars().collect();
    let mut search_from = 0;

    for q_ch in query_lower.chars() {
        let mut found_at = None;
        for (idx, &c_ch) in cand_chars.iter().enumerate().skip(search_from) {
            if c_ch == q_ch {
                found_at = Some(idx);
                break;
            }
        }
        let idx = found_at?;
        if let Some(prev) = last_match_idx {
            if idx == prev + 1 {
                score += 35; // Consecutive character bonus
            } else {
                score -= (idx - prev) as i64;
            }
        }
        if idx == 0 || matches!(cand_chars.get(idx.wrapping_sub(1)), Some('/' | '_' | '-' | '.')) {
            score += 25; // Word/segment boundary bonus
        }
        last_match_idx = Some(idx);
        search_from = idx + 1;
    }

    Some(score - candidate.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_gitignore_aware_scanning_and_plan_detection() {
        let tmp_root = std::env::temp_dir().join(format!(
            "verbot_files_scan_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp_root);
        fs::create_dir_all(tmp_root.join(".git")).unwrap();
        fs::create_dir_all(tmp_root.join("src")).unwrap();
        fs::create_dir_all(tmp_root.join("ignored_dir")).unwrap();
        fs::create_dir_all(tmp_root.join("target/debug")).unwrap();

        fs::write(tmp_root.join(".gitignore"), "ignored_dir/\n*.secret.log\n").unwrap();
        fs::write(tmp_root.join("STEPPED_IMPLEMENTATION_PLAN.md"), "# Plan\n").unwrap();
        fs::write(tmp_root.join("README.md"), "# Readme\n").unwrap();
        fs::write(tmp_root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(tmp_root.join("src/theme.rs"), "pub struct Theme;\n").unwrap();
        fs::write(tmp_root.join("ignored_dir/should_not_appear.rs"), "").unwrap();
        fs::write(tmp_root.join("debug.secret.log"), "secret").unwrap();
        fs::write(tmp_root.join("target/debug/verbot-tui"), "bin").unwrap();

        let index = WorkspaceFileIndex::scan(&tmp_root).expect("scan should succeed");
        let rel_paths: Vec<&str> = index.files.iter().map(|f| f.relative_path.as_str()).collect();

        assert!(rel_paths.contains(&"STEPPED_IMPLEMENTATION_PLAN.md"));
        assert!(rel_paths.contains(&"README.md"));
        assert!(rel_paths.contains(&"src/main.rs"));
        assert!(rel_paths.contains(&"src/theme.rs"));
        assert!(!rel_paths.contains(&"ignored_dir/should_not_appear.rs"));
        assert!(!rel_paths.contains(&"debug.secret.log"));
        assert!(!rel_paths.contains(&"target/debug/verbot-tui"));

        let primary_plan = index
            .detect_primary_plan_file()
            .expect("should detect STEPPED_IMPLEMENTATION_PLAN.md");
        assert_eq!(primary_plan.relative_path, "STEPPED_IMPLEMENTATION_PLAN.md");

        let fuzzy_theme = index.fuzzy_search("thm", false, 5);
        assert_eq!(fuzzy_theme[0].relative_path, "src/theme.rs");

        let fuzzy_md = index.fuzzy_search("", true, 5);
        assert_eq!(fuzzy_md[0].relative_path, "STEPPED_IMPLEMENTATION_PLAN.md");

        let _ = fs::remove_dir_all(&tmp_root);
    }
}
