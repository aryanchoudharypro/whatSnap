use std::time::Duration;

use serde::{Deserialize, Serialize};

const GITHUB_REPO: &str = "aryanchoudharypro/whatSnap";
const CACHE_FILE_NAME: &str = "whats_new_cache.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseChange {
	pub tag_name: String,
	pub name: String,
	pub published_at: String,
	pub body: String,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseApi {
	tag_name: Option<String>,
	name: Option<String>,
	published_at: Option<String>,
	body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubCommitApi {
	sha: Option<String>,
	commit: Option<CommitDetails>,
}

#[derive(Debug, Deserialize)]
struct CommitDetails {
	message: Option<String>,
	author: Option<CommitAuthor>,
}

#[derive(Debug, Deserialize)]
struct CommitAuthor {
	date: Option<String>,
}

fn cache_path() -> std::path::PathBuf {
	crate::platform::paths::app_data_dir().join(CACHE_FILE_NAME)
}

pub fn load_cached() -> Option<Vec<ReleaseChange>> {
	let path = cache_path();
	if path.exists() {
		if let Ok(content) = std::fs::read_to_string(&path) {
			if let Ok(changes) = serde_json::from_str::<Vec<ReleaseChange>>(&content) {
				if !changes.is_empty() {
					return Some(changes);
				}
			}
		}
	}
	None
}

pub fn save_cache(changes: &[ReleaseChange]) {
	let path = cache_path();
	if let Ok(data) = serde_json::to_string_pretty(changes) {
		let _ = std::fs::write(path, data);
	}
}

pub fn embedded_changelog() -> Vec<ReleaseChange> {
	vec![
		ReleaseChange {
			tag_name: "v0.1.2".to_string(),
			name: "whatSnap Multi-Account & Enhanced Experience".to_string(),
			published_at: "2026-11-06".to_string(),
			body: [
				"### What's New:",
				"- Multi-Account Support: Link and switch between multiple WhatsApp accounts seamlessly with Account Switcher (Ctrl+Shift+A).",
				"- Strict Session Isolation: Zero notification or alert bleed between active and dormant accounts.",
				"- Live What's New viewer: Fetches real-time commits and release notes directly from GitHub.",
				"- Full Screen-Reader Accessibility: Complete NVDA/JAWS live region announcements and high-contrast styling across all account and changelog panels.",
				"- App Lock: Secure whatSnap with Windows Hello biometric verification or PIN/Password protection.",
				"- WIN_HYBRID Companion Protocol parity: Ultra-low latency voice and video calling with native WASAPI and Opus DSP.",
				"- Group status update viewer with WhatsApp dark styling.",
				"- Native call audio processing with custom noise suppressor, automatic gain control and limiter.",
			].join("\n"),
		},
		ReleaseChange {
			tag_name: "v0.1.1".to_string(),
			name: "whatSnap Native Desktop Release".to_string(),
			published_at: "2026-11-04".to_string(),
			body: [
				"### Features:",
				"- Keyboard-first native Windows client using wxWidgets and AccessKit.",
				"- Offline SQLite session history cache.",
				"- High throughput duplex Win32 Named Pipes IPC.",
				"- Rich attachment composer with pasted media support.",
				"- Full support for WhatsApp voice messages with WSOLA speed pitching.",
			].join("\n"),
		},
	]
}

pub fn fetch_from_github() -> Result<Vec<ReleaseChange>, String> {
	let client = ureq::Agent::new_with_config(
		ureq::config::Config::builder()
			.timeout_connect(Some(Duration::from_secs(8)))
			.timeout_global(Some(Duration::from_secs(12)))
			.build(),
	);

	let mut results = Vec::new();

	// 1. Try fetching all releases
	let releases_url = format!("https://api.github.com/repos/{}/releases", GITHUB_REPO);
	if let Ok(mut resp) = client
		.get(&releases_url)
		.header("User-Agent", "whatSnap")
		.header("Accept", "application/vnd.github+json")
		.call()
	{
		if let Ok(releases) = resp.body_mut().read_json::<Vec<GithubReleaseApi>>() {
			for r in releases {
				let tag = r.tag_name.unwrap_or_else(|| "latest".to_string());
				let name = r.name.unwrap_or_else(|| tag.clone());
				let pub_date = r.published_at.unwrap_or_default();
				let date_str = if pub_date.len() >= 10 { pub_date[..10].to_string() } else { pub_date };
				let body = r.body.unwrap_or_default();
				if !body.trim().is_empty() {
					results.push(ReleaseChange { tag_name: tag, name, published_at: date_str, body });
				}
			}
		}
	}

	// 2. If releases were empty or only had "latest", check tag "latest" specifically
	if results.is_empty() {
		let latest_url = format!("https://api.github.com/repos/{}/releases/tags/latest", GITHUB_REPO);
		if let Ok(mut resp) = client
			.get(&latest_url)
			.header("User-Agent", "whatSnap")
			.header("Accept", "application/vnd.github+json")
			.call()
		{
			if let Ok(r) = resp.body_mut().read_json::<GithubReleaseApi>() {
				let tag = r.tag_name.unwrap_or_else(|| "latest".to_string());
				let name = r.name.unwrap_or_else(|| "Latest Development Build".to_string());
				let pub_date = r.published_at.unwrap_or_default();
				let date_str = if pub_date.len() >= 10 { pub_date[..10].to_string() } else { pub_date };
				let body = r.body.unwrap_or_default();
				if !body.trim().is_empty() {
					results.push(ReleaseChange { tag_name: tag, name, published_at: date_str, body });
				}
			}
		}
	}

	// 3. Also fetch recent commits from GitHub if needed to provide a detailed changelog
	let commits_url = format!("https://api.github.com/repos/{}/commits?per_page=40", GITHUB_REPO);
	if let Ok(mut resp) =
		client.get(&commits_url).header("User-Agent", "whatSnap").header("Accept", "application/vnd.github+json").call()
	{
		if let Ok(commits) = resp.body_mut().read_json::<Vec<GithubCommitApi>>() {
			let mut commit_lines = Vec::new();
			for c in commits {
				let sha = c.sha.map(|s| if s.len() >= 7 { s[..7].to_string() } else { s }).unwrap_or_default();
				let msg = c
					.commit
					.as_ref()
					.and_then(|d| d.message.as_ref())
					.map(|m| m.lines().next().unwrap_or("").to_string())
					.unwrap_or_default();
				let date = c
					.commit
					.as_ref()
					.and_then(|d| d.author.as_ref())
					.and_then(|a| a.date.as_ref())
					.map(|dt| if dt.len() >= 10 { dt[..10].to_string() } else { dt.clone() })
					.unwrap_or_default();

				if !msg.is_empty() {
					if !date.is_empty() {
						commit_lines.push(format!("- [{}] ({}) {}", sha, date, msg));
					} else {
						commit_lines.push(format!("- [{}] {}", sha, msg));
					}
				}
			}

			if !commit_lines.is_empty() {
				results.push(ReleaseChange {
					tag_name: "git-commits".to_string(),
					name: "Recent Commits History".to_string(),
					published_at: "Live from GitHub".to_string(),
					body: format!("## Recent Codebase Commits (from GitHub master):\n\n{}", commit_lines.join("\n")),
				});
			}
		}
	}

	if results.is_empty() {
		Err("Could not retrieve changes from GitHub. Check your network connection.".to_string())
	} else {
		save_cache(&results);
		Ok(results)
	}
}

pub fn get_whats_new() -> Vec<ReleaseChange> {
	if let Ok(fresh) = fetch_from_github() {
		return fresh;
	}
	if let Some(cached) = load_cached() {
		return cached;
	}
	embedded_changelog()
}
