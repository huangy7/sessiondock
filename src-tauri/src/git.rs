use notify::{recommended_watcher, Event, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

// ─── Serialized response types ──────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    /// "M" | "A" | "D" | "U" | "R" | "?" (untracked)
    pub status: String,
    pub staged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlameLineInfo {
    pub line: usize,
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFileContent {
    pub original: String,
    pub modified: String,
    pub too_large: bool,
}

// ─── Blame cache (Tauri managed state) ──────────────────────────────

/// Key: (repo_path, file_path, head_commit_hash)
pub type BlameCacheState = Mutex<HashMap<(String, String, String), Vec<BlameLineInfo>>>;

// ─── Git watcher (Tauri managed state) ─────────────────────────────

pub struct GitWatcherEntry {
    pub stop_flag: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub watcher: Box<dyn Watcher + Send>,
}

pub type GitWatcherState = Mutex<HashMap<String, GitWatcherEntry>>;

static GIT_WATCHER_STOP_FLAGS: LazyLock<Mutex<Vec<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

pub fn cleanup_watchers() {
    if let Ok(flags) = GIT_WATCHER_STOP_FLAGS.lock() {
        for flag in flags.iter() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

// ─── Helper: open repo in spawn_blocking ────────────────────────────

fn open_repo(path: &str) -> Result<git2::Repository, String> {
    git2::Repository::discover(path).map_err(|e| format!("无法打开 Git 仓库: {}", e))
}

fn head_commit_hash(repo: &git2::Repository) -> Result<String, String> {
    let head = repo.head().map_err(|e| format!("无法读取 HEAD: {}", e))?;
    let commit = head.peel_to_commit().map_err(|e| format!("无法解析 HEAD commit: {}", e))?;
    Ok(commit.id().to_string())
}

fn status_char(s: git2::Status) -> &'static str {
    if s.contains(git2::Status::IGNORED) { return "I"; }
    if s.contains(git2::Status::INDEX_NEW) || s.contains(git2::Status::WT_NEW) { return "A"; }
    if s.contains(git2::Status::INDEX_DELETED) || s.contains(git2::Status::WT_DELETED) { return "D"; }
    if s.contains(git2::Status::INDEX_RENAMED) || s.contains(git2::Status::WT_RENAMED) { return "R"; }
    if s.contains(git2::Status::INDEX_MODIFIED) || s.contains(git2::Status::WT_MODIFIED) { return "M"; }
    "?"
}

// ─── Commands ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn git_status(path: String, include_ignored: Option<bool>) -> Result<Vec<GitStatusEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true);
        if include_ignored.unwrap_or(false) {
            opts.include_ignored(true);
        } else {
            opts.include_ignored(false);
        }
        let statuses = repo.statuses(Some(&mut opts))
            .map_err(|e| format!("读取 git status 失败: {}", e))?;

        let mut entries = Vec::new();
        for entry in statuses.iter() {
            let file_path = entry.path().unwrap_or("").to_string();
            let s = entry.status();
            let staged = s.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED,
            );
            entries.push(GitStatusEntry {
                path: file_path,
                status: status_char(s).to_string(),
                staged,
            });
        }
        Ok(entries)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_log(path: String, file: Option<String>, limit: Option<usize>) -> Result<Vec<CommitInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let mut revwalk = repo.revwalk().map_err(|e| format!("revwalk 失败: {}", e))?;
        revwalk.push_head().map_err(|e| format!("push_head 失败: {}", e))?;
        revwalk.set_sorting(git2::Sort::TIME).map_err(|e| format!("排序失败: {}", e))?;

        let limit = limit.unwrap_or(100);
        let mut commits = Vec::new();

        for oid_result in revwalk {
            if commits.len() >= limit { break; }
            let oid = oid_result.map_err(|e| format!("revwalk error: {}", e))?;
            let commit = repo.find_commit(oid).map_err(|e| format!("find_commit: {}", e))?;

            // If filtering by file, check if this commit touches it
            if let Some(ref file_path) = file {
                let dominated = commit_touches_file(&repo, &commit, file_path);
                if !dominated { continue; }
            }

            let author = commit.author();
            commits.push(CommitInfo {
                hash: oid.to_string(),
                short_hash: oid.to_string()[..7.min(oid.to_string().len())].to_string(),
                message: commit.message().unwrap_or("").to_string(),
                author: author.name().unwrap_or("").to_string(),
                email: author.email().unwrap_or("").to_string(),
                timestamp: author.when().seconds(),
            });
        }
        Ok(commits)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

fn commit_touches_file(repo: &git2::Repository, commit: &git2::Commit, file_path: &str) -> bool {
    let tree = match commit.tree() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(git2::DiffOptions::new().pathspec(file_path)),
    );
    match diff {
        Ok(d) => d.deltas().count() > 0,
        Err(_) => false,
    }
}

#[tauri::command]
pub async fn git_branches(path: String) -> Result<Vec<BranchInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let branches = repo.branches(None).map_err(|e| format!("列出分支失败: {}", e))?;
        let head = repo.head().ok();
        let head_name = head.as_ref().and_then(|h| h.shorthand().map(|s| s.to_string()));

        let mut result = Vec::new();
        for branch_result in branches {
            let (branch, branch_type) = branch_result.map_err(|e| format!("branch error: {}", e))?;
            let name = branch.name().ok().flatten().unwrap_or("").to_string();
            let is_remote = branch_type == git2::BranchType::Remote;
            let is_current = !is_remote && head_name.as_deref() == Some(&name);
            let upstream = branch.upstream().ok().and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
            result.push(BranchInfo { name, is_current, is_remote, upstream });
        }
        Ok(result)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_is_repo(path: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(git2::Repository::discover(&path).is_ok())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

// ─── Phase 2: Diff / Stage / Commit commands ──────────────────────────

const MAX_DIFF_SIZE: usize = 1024 * 1024; // 1MB

#[derive(Debug, Clone, Serialize)]
pub struct CommitDiffFile {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitDiffResult {
    pub info: CommitInfo,
    pub files: Vec<CommitDiffFile>,
}

#[tauri::command]
pub async fn git_diff_file_content(path: String, file: String) -> Result<DiffFileContent, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let workdir = repo.workdir().ok_or("仓库无工作目录")?;
        let abs_file = workdir.join(&file);

        // Read working tree version
        let modified = std::fs::read_to_string(&abs_file)
            .map_err(|e| format!("读取工作区文件失败: {}", e))?;
        if modified.len() > MAX_DIFF_SIZE {
            return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
        }

        // Read HEAD version
        let head = repo.head().map_err(|e| format!("读取 HEAD 失败: {}", e))?;
        let tree = head.peel_to_tree().map_err(|e| format!("解析 tree 失败: {}", e))?;
        let entry = tree.get_path(Path::new(&file));
        let original = match entry {
            Ok(e) => {
                let blob = repo.find_blob(e.id()).map_err(|e| format!("读取 blob 失败: {}", e))?;
                if blob.size() > MAX_DIFF_SIZE {
                    return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
                }
                String::from_utf8_lossy(blob.content()).to_string()
            }
            Err(_) => String::new(), // New file
        };

        Ok(DiffFileContent { original, modified, too_large: false })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_commit_diff(path: String, hash: String) -> Result<CommitDiffResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let oid = git2::Oid::from_str(&hash).map_err(|e| format!("无效的 commit hash: {}", e))?;
        let commit = repo.find_commit(oid).map_err(|e| format!("找不到 commit: {}", e))?;
        let tree = commit.tree().map_err(|e| format!("解析 tree 失败: {}", e))?;
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(|e| format!("diff 失败: {}", e))?;

        let mut files = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                let file_path = delta.new_file().path()
                    .or_else(|| delta.old_file().path())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let status = match delta.status() {
                    git2::Delta::Added => "A",
                    git2::Delta::Deleted => "D",
                    git2::Delta::Modified => "M",
                    git2::Delta::Renamed => "R",
                    _ => "?",
                };
                files.push(CommitDiffFile { path: file_path, status: status.to_string() });
                true
            },
            None, None, None,
        ).map_err(|e| format!("diff foreach 失败: {}", e))?;

        let author = commit.author();
        let hash_str = oid.to_string();
        let info = CommitInfo {
            hash: hash_str.clone(),
            short_hash: hash_str[..7.min(hash_str.len())].to_string(),
            message: commit.message().unwrap_or("").to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            timestamp: author.when().seconds(),
        };

        Ok(CommitDiffResult { info, files })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_commit_file_diff(path: String, hash: String, file: String) -> Result<DiffFileContent, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let oid = git2::Oid::from_str(&hash).map_err(|e| format!("无效 hash: {}", e))?;
        let commit = repo.find_commit(oid).map_err(|e| format!("找不到 commit: {}", e))?;
        let tree = commit.tree().map_err(|e| format!("tree 失败: {}", e))?;

        // New version (this commit)
        let modified = match tree.get_path(Path::new(&file)) {
            Ok(e) => {
                let blob = repo.find_blob(e.id()).map_err(|e| format!("blob 失败: {}", e))?;
                if blob.size() > MAX_DIFF_SIZE {
                    return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
                }
                String::from_utf8_lossy(blob.content()).to_string()
            }
            Err(_) => String::new(), // File deleted in this commit
        };

        // Old version (parent commit)
        let original = match commit.parent(0) {
            Ok(parent) => {
                let parent_tree = parent.tree().map_err(|e| format!("parent tree 失败: {}", e))?;
                match parent_tree.get_path(Path::new(&file)) {
                    Ok(e) => {
                        let blob = repo.find_blob(e.id()).map_err(|e| format!("blob 失败: {}", e))?;
                        if blob.size() > MAX_DIFF_SIZE {
                            return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
                        }
                        String::from_utf8_lossy(blob.content()).to_string()
                    }
                    Err(_) => String::new(),
                }
            }
            Err(_) => String::new(), // Initial commit
        };

        Ok(DiffFileContent { original, modified, too_large: false })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_stage(path: String, files: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let mut index = repo.index().map_err(|e| format!("读取 index 失败: {}", e))?;
        for file in &files {
            let workdir = repo.workdir().ok_or("仓库无工作目录")?;
            if workdir.join(file).exists() {
                index.add_path(Path::new(file)).map_err(|e| format!("暂存失败: {}", e))?;
            } else {
                index.remove_path(Path::new(file)).map_err(|e| format!("暂存删除失败: {}", e))?;
            }
        }
        index.write().map_err(|e| format!("写入 index 失败: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_unstage(path: String, files: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let head = repo.head().and_then(|h| h.peel_to_commit()).ok();
        let head_obj = head.as_ref().map(|c| c.as_object());
        repo.reset_default(head_obj, files.iter().map(Path::new))
            .map_err(|e| format!("取消暂存失败: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_commit(path: String, message: String) -> Result<CommitInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let sig = repo.signature().map_err(|_| {
            "无法获取 Git 用户信息。请先配置 git config user.name 和 user.email".to_string()
        })?;
        let mut index = repo.index().map_err(|e| format!("读取 index 失败: {}", e))?;
        let tree_oid = index.write_tree().map_err(|e| format!("写入 tree 失败: {}", e))?;
        let tree = repo.find_tree(tree_oid).map_err(|e| format!("查找 tree 失败: {}", e))?;

        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();

        let oid = repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parents)
            .map_err(|e| format!("提交失败: {}", e))?;

        let hash_str = oid.to_string();
        Ok(CommitInfo {
            hash: hash_str.clone(),
            short_hash: hash_str[..7.min(hash_str.len())].to_string(),
            message,
            author: sig.name().unwrap_or("").to_string(),
            email: sig.email().unwrap_or("").to_string(),
            timestamp: sig.when().seconds(),
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

// ─── Phase 3: Branch ops / Network CLI / Blame ────────────────────────

#[tauri::command]
pub async fn git_checkout(path: String, branch: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;

        // Safety check: abort if workdir has uncommitted changes
        let statuses = repo.statuses(None).map_err(|e| format!("status 失败: {}", e))?;
        let has_dirty = statuses.iter().any(|e| {
            let s = e.status();
            s.intersects(
                git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED,
            )
        });
        if has_dirty {
            return Err("工作区有未提交的修改，请先提交或暂存后再切换分支".to_string());
        }

        let (object, reference) = repo.revparse_ext(&branch)
            .map_err(|e| format!("解析分支失败: {}", e))?;
        repo.checkout_tree(&object, None)
            .map_err(|e| format!("checkout 失败: {}", e))?;
        match reference {
            Some(r) => {
                let refname = r.name().ok_or("无效的引用名")?;
                repo.set_head(refname).map_err(|e| format!("设置 HEAD 失败: {}", e))?;
            }
            None => {
                repo.set_head_detached(object.id())
                    .map_err(|e| format!("设置 HEAD 失败: {}", e))?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_create_branch(path: String, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let head = repo.head().map_err(|e| format!("HEAD 失败: {}", e))?;
        let commit = head.peel_to_commit().map_err(|e| format!("commit 失败: {}", e))?;
        repo.branch(&name, &commit, false)
            .map_err(|e| format!("创建分支失败: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_delete_branch(path: String, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let mut branch = repo.find_branch(&name, git2::BranchType::Local)
            .map_err(|e| format!("找不到分支: {}", e))?;
        branch.delete().map_err(|e| format!("删除分支失败: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

fn run_git_cli(path: &str, args: &[&str]) -> Result<String, String> {
    #[allow(unused_mut)]
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().map_err(|e| format!("执行 git 命令失败: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
pub async fn git_push(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || run_git_cli(&path, &["push"]))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_pull(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || run_git_cli(&path, &["pull"]))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_fetch(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || run_git_cli(&path, &["fetch", "--all"]))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn git_blame(
    path: String,
    file: String,
    state: tauri::State<'_, BlameCacheState>,
) -> Result<Vec<BlameLineInfo>, String> {
    // Check cache first
    let cache_key = {
        let repo = git2::Repository::discover(&path)
            .map_err(|e| format!("无法打开仓库: {}", e))?;
        let head_hash = head_commit_hash(&repo)?;
        (path.clone(), file.clone(), head_hash)
    };

    {
        let cache = state.lock().map_err(|_| "缓存锁失败")?;
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached.clone());
        }
    }

    let key = cache_key.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&key.0)?;

        // Large file protection: read file and check line count
        let workdir = repo.workdir().ok_or("仓库无工作目录")?;
        let abs_path = workdir.join(&key.1);
        let content = std::fs::read_to_string(&abs_path)
            .map_err(|e| format!("读取文件失败: {}", e))?;
        if content.lines().count() > 5000 {
            return Err("FILE_TOO_LARGE".to_string());
        }

        let blame = repo.blame_file(Path::new(&key.1), None)
            .map_err(|e| format!("blame 失败: {}", e))?;

        let mut lines = Vec::new();
        for (i, _line) in content.lines().enumerate() {
            let hunk = blame.get_line(i + 1); // blame is 1-indexed
            if let Some(hunk) = hunk {
                let sig = hunk.final_signature();
                lines.push(BlameLineInfo {
                    line: i + 1,
                    hash: hunk.final_commit_id().to_string(),
                    short_hash: hunk.final_commit_id().to_string()[..7].to_string(),
                    author: sig.name().unwrap_or("").to_string(),
                    email: sig.email().unwrap_or("").to_string(),
                    timestamp: sig.when().seconds(),
                    message: String::new(), // Filled below
                });
            }
        }

        // Batch-fetch commit messages for unique hashes
        let unique_hashes: Vec<String> = lines.iter().map(|l| l.hash.clone()).collect::<std::collections::HashSet<_>>().into_iter().collect();
        let mut msg_map: HashMap<String, String> = HashMap::new();
        for hash in &unique_hashes {
            if let Ok(oid) = git2::Oid::from_str(hash) {
                if let Ok(commit) = repo.find_commit(oid) {
                    let msg = commit.summary().unwrap_or("").to_string();
                    msg_map.insert(hash.clone(), msg);
                }
            }
        }
        for line in &mut lines {
            if let Some(msg) = msg_map.get(&line.hash) {
                line.message = msg.clone();
            }
        }

        Ok(lines)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))??;

    // Store in cache
    {
        let mut cache = state.lock().map_err(|_| "缓存锁失败")?;
        cache.insert(cache_key, result.clone());
    }

    Ok(result)
}

#[tauri::command]
pub async fn git_file_history(path: String, file: String, limit: Option<u32>) -> Result<Vec<CommitInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let max_count = limit.unwrap_or(100);
        #[allow(unused_mut)]
        let mut cmd = Command::new("git");
        cmd.args(["log", "--follow", &format!("--max-count={}", max_count), "--format=%H%n%h%n%an%n%ae%n%at%n%s", "--", &file])
            .current_dir(&path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output().map_err(|e| format!("git log --follow 失败: {}", e))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut commits = Vec::new();
        let lines: Vec<&str> = stdout.lines().collect();
        for chunk in lines.chunks(6) {
            if chunk.len() < 6 { break; }
            commits.push(CommitInfo {
                hash: chunk[0].to_string(),
                short_hash: chunk[1].to_string(),
                author: chunk[2].to_string(),
                email: chunk[3].to_string(),
                timestamp: chunk[4].parse().unwrap_or(0),
                message: chunk[5].to_string(),
            });
        }
        Ok(commits)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

// ─── Phase 4: Git state watching ────────────────────────────────────

#[tauri::command]
pub fn watch_git_state(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, GitWatcherState>,
) -> Result<(), String> {
    use tauri::Emitter;
    let mut guard = state.lock().map_err(|_| "git watcher 锁失败")?;

    // Remove old watcher
    if let Some(old) = guard.remove(&path) {
        old.stop_flag.store(true, Ordering::SeqCst);
    }

    // Find .git dir
    let repo = git2::Repository::discover(&path)
        .map_err(|e| format!("不是 Git 仓库: {}", e))?;
    let git_dir = repo.path().to_path_buf(); // .git/
    drop(repo);

    let head_path = git_dir.join("HEAD");
    let index_path = git_dir.join("index");

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = recommended_watcher(tx)
        .map_err(|e| format!("创建 git watcher 失败: {}", e))?;

    // Watch .git/HEAD and .git/index
    if head_path.exists() {
        watcher.watch(&head_path, RecursiveMode::NonRecursive)
            .map_err(|e| format!("监听 HEAD 失败: {}", e))?;
    }
    if index_path.exists() {
        watcher.watch(&index_path, RecursiveMode::NonRecursive)
            .map_err(|e| format!("监听 index 失败: {}", e))?;
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_flag.clone();
    let path_clone = path.clone();

    if let Ok(mut flags) = GIT_WATCHER_STOP_FLAGS.lock() {
        flags.push(stop_flag.clone());
    }

    std::thread::spawn(move || {
        let debounce = Duration::from_millis(500);
        let mut last_event: Option<Instant> = None;
        loop {
            if stop_clone.load(Ordering::SeqCst) { break; }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(_event)) => {
                    last_event = Some(Instant::now());
                }
                Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(last) = last_event {
                if last.elapsed() >= debounce {
                    last_event = None;
                    let _ = app.emit("git-state-changed", &path_clone);
                }
            }
        }
    });

    guard.insert(path, GitWatcherEntry {
        stop_flag,
        watcher: Box::new(watcher),
    });

    Ok(())
}

#[tauri::command]
pub fn unwatch_git_state(
    path: String,
    state: tauri::State<'_, GitWatcherState>,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|_| "git watcher 锁失败")?;
    if let Some(entry) = guard.remove(&path) {
        entry.stop_flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}
