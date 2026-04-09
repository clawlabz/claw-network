//! Version checking and upgrade level determination.
//!
//! Fetches version manifest from GitHub and checks if an upgrade is needed.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Version manifest downloaded from GitHub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: String,
    pub minimum: String,
    #[serde(default)]
    pub critical_minimum: Option<String>,
    pub release_url: String,
    pub changelog: String,
    #[serde(default)]
    pub announcement: Option<String>,
    #[serde(default)]
    pub halt_height: Option<u64>,
    #[serde(default)]
    pub halt_message: Option<String>,
    #[serde(default)]
    pub plugin_latest: Option<String>,
    #[serde(default)]
    pub plugin_minimum: Option<String>,
    #[serde(default)]
    pub plugin_changelog: Option<String>,
    pub updated_at: String,
}

/// Upgrade level determined by comparing current version to manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeLevel {
    UpToDate,
    Recommended,
    Required,
    Critical,
}

impl UpgradeLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UpToDate => "up_to_date",
            Self::Recommended => "recommended",
            Self::Required => "required",
            Self::Critical => "critical",
        }
    }
}

/// Fetch version manifest from GitHub with a 5-second timeout.
/// Returns None on any network error or timeout (never blocks startup).
pub async fn fetch_manifest() -> Option<VersionManifest> {
    let url = "https://raw.githubusercontent.com/clawlabz/claw-network/main/version-manifest.json";
    let timeout = Duration::from_secs(5);

    match tokio::time::timeout(timeout, reqwest::Client::new().get(url).send()).await {
        Ok(Ok(response)) => {
            match response.json::<VersionManifest>().await {
                Ok(manifest) => Some(manifest),
                Err(e) => {
                    tracing::warn!("Failed to parse manifest JSON: {e}");
                    None
                }
            }
        }
        Ok(Err(e)) => {
            tracing::debug!("Failed to fetch manifest: {e}");
            None
        }
        Err(_) => {
            tracing::debug!("Manifest fetch timed out after 5s");
            None
        }
    }
}

/// Compare semantic versions as strings (e.g., "0.4.19").
/// Splits on '.' and compares numerically. Returns true if lhs < rhs.
fn version_less_than(lhs: &str, rhs: &str) -> bool {
    let lhs_parts: Vec<u32> = lhs.split('.').filter_map(|s| s.parse().ok()).collect();
    let rhs_parts: Vec<u32> = rhs.split('.').filter_map(|s| s.parse().ok()).collect();

    for i in 0..lhs_parts.len().max(rhs_parts.len()) {
        let l = lhs_parts.get(i).copied().unwrap_or(0);
        let r = rhs_parts.get(i).copied().unwrap_or(0);
        if l < r {
            return true;
        } else if l > r {
            return false;
        }
    }
    false
}

/// Determine upgrade level based on current version, manifest, and current block height.
/// Logic:
/// - If current < critical_minimum: Critical
/// - If halt_height is Some and current_height >= halt_height: Critical
/// - If current < minimum: Required
/// - If current < latest: Recommended
/// - Else: UpToDate
pub fn check_version(
    current: &str,
    manifest: &VersionManifest,
    current_height: Option<u64>,
) -> UpgradeLevel {
    // Check critical_minimum
    if let Some(ref critical) = manifest.critical_minimum {
        if version_less_than(current, critical) {
            return UpgradeLevel::Critical;
        }
    }

    // Check halt_height
    if let Some(halt_height) = manifest.halt_height {
        if let Some(height) = current_height {
            if height >= halt_height {
                return UpgradeLevel::Critical;
            }
        }
    }

    // Check minimum
    if version_less_than(current, &manifest.minimum) {
        return UpgradeLevel::Required;
    }

    // Check latest
    if version_less_than(current, &manifest.latest) {
        return UpgradeLevel::Recommended;
    }

    UpgradeLevel::UpToDate
}

/// Format an upgrade message for CLI output.
pub fn format_upgrade_message(level: &UpgradeLevel, manifest: &VersionManifest) -> String {
    match level {
        UpgradeLevel::UpToDate => "Node is up to date.".to_string(),
        UpgradeLevel::Recommended => {
            format!(
                "Update available: {} (current: {}). {} Download: {}",
                manifest.latest,
                env!("CARGO_PKG_VERSION"),
                manifest.changelog,
                manifest.release_url
            )
        }
        UpgradeLevel::Required => {
            format!(
                "Upgrade required: minimum version {}. Current: {}. {}. Download: {}",
                manifest.minimum,
                env!("CARGO_PKG_VERSION"),
                manifest.changelog,
                manifest.release_url
            )
        }
        UpgradeLevel::Critical => {
            let reason = if let Some(ref crit) = manifest.critical_minimum {
                format!(
                    "Critical update required. Minimum version: {}. Current: {}.",
                    crit,
                    env!("CARGO_PKG_VERSION")
                )
            } else if manifest.halt_height.is_some() {
                format!(
                    "Node has reached halt height. Upgrade required to continue. Download: {}",
                    manifest.release_url
                )
            } else {
                "Critical update required".to_string()
            };
            format!(
                "{}. Changelog: {}. {}",
                reason, manifest.changelog, manifest.release_url
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(version_less_than("0.4.18", "0.4.19"));
        assert!(version_less_than("0.3.99", "0.4.0"));
        assert!(!version_less_than("0.4.19", "0.4.19"));
        assert!(!version_less_than("0.5.0", "0.4.19"));
    }

    #[test]
    fn test_upgrade_level_critical_minimum() {
        let manifest = VersionManifest {
            latest: "0.5.0".to_string(),
            minimum: "0.4.19".to_string(),
            critical_minimum: Some("0.5.0".to_string()),
            release_url: "https://example.com".to_string(),
            changelog: "test".to_string(),
            announcement: None,
            halt_height: None,
            halt_message: None,
            updated_at: "2026-04-03T00:00:00Z".to_string(),
            plugin_latest: None,
            plugin_minimum: None,
            plugin_changelog: None,
        };
        assert_eq!(check_version("0.4.19", &manifest, None), UpgradeLevel::Critical);
        assert_eq!(check_version("0.5.0", &manifest, None), UpgradeLevel::UpToDate);
    }

    #[test]
    fn test_upgrade_level_halt_height() {
        let manifest = VersionManifest {
            latest: "0.5.0".to_string(),
            minimum: "0.4.19".to_string(),
            critical_minimum: None,
            release_url: "https://example.com".to_string(),
            changelog: "test".to_string(),
            announcement: None,
            halt_height: Some(300000),
            halt_message: Some("Halt at 300000".to_string()),
            updated_at: "2026-04-03T00:00:00Z".to_string(),
            plugin_latest: None,
            plugin_minimum: None,
            plugin_changelog: None,
        };
        assert_eq!(check_version("0.4.19", &manifest, Some(250000)), UpgradeLevel::Recommended);
        assert_eq!(check_version("0.4.19", &manifest, Some(300000)), UpgradeLevel::Critical);
        assert_eq!(check_version("0.4.19", &manifest, Some(300001)), UpgradeLevel::Critical);
    }

    #[test]
    fn test_upgrade_level_required() {
        let manifest = VersionManifest {
            latest: "0.5.0".to_string(),
            minimum: "0.4.19".to_string(),
            critical_minimum: None,
            release_url: "https://example.com".to_string(),
            changelog: "test".to_string(),
            announcement: None,
            halt_height: None,
            halt_message: None,
            updated_at: "2026-04-03T00:00:00Z".to_string(),
            plugin_latest: None,
            plugin_minimum: None,
            plugin_changelog: None,
        };
        assert_eq!(check_version("0.4.18", &manifest, None), UpgradeLevel::Required);
    }
}
