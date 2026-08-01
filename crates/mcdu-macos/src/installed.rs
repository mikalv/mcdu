use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Error when installed-app discovery fails critically.
#[derive(Debug, Clone)]
pub struct InstalledAppsError(pub String);

/// Discover all installed application bundle IDs using Spotlight (mdfind).
/// Returns Err if mdfind fails or finds zero apps (likely Spotlight disabled),
/// so callers can abort orphan scanning instead of flagging everything.
pub fn get_installed_bundle_ids() -> Result<HashSet<String>, InstalledAppsError> {
    let mut ids = HashSet::new();

    let output = Command::new("mdfind")
        .arg("kMDItemContentType == 'com.apple.application-bundle'")
        .output()
        .map_err(|e| InstalledAppsError(format!("mdfind failed to start: {e}")))?;

    if !output.status.success() {
        return Err(InstalledAppsError(format!(
            "mdfind exited with status {}",
            output.status
        )));
    }

    let app_paths = String::from_utf8_lossy(&output.stdout);
    let mut app_count = 0usize;

    for app_path in app_paths.lines() {
        let app_path = app_path.trim();
        if app_path.is_empty() {
            continue;
        }
        app_count += 1;

        if let Some(bundle_id) = read_bundle_id(app_path) {
            ids.insert(bundle_id);
        }
    }

    if app_count == 0 {
        return Err(InstalledAppsError(
            "mdfind returned 0 apps — Spotlight may be disabled or broken".into(),
        ));
    }

    Ok(ids)
}

/// Read CFBundleIdentifier from an .app bundle's Info.plist via plutil (fast, no per-app defaults).
fn read_bundle_id(app_path: &str) -> Option<String> {
    let plist_path = format!("{}/Contents/Info.plist", app_path);
    if !Path::new(&plist_path).exists() {
        return None;
    }

    // Prefer plutil extract (no process-per-defaults overhead of parsing whole plist via defaults)
    if let Ok(output) = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-", &plist_path])
        .output()
    {
        if output.status.success() {
            let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }

    // Fallback: simple XML/text scan for CFBundleIdentifier string value
    if let Ok(contents) = fs::read_to_string(&plist_path) {
        return extract_bundle_id_from_plist_text(&contents);
    }

    None
}

fn extract_bundle_id_from_plist_text(contents: &str) -> Option<String> {
    // Works for XML plists: <key>CFBundleIdentifier</key>\n\t<string>com.foo</string>
    let key = "CFBundleIdentifier";
    let bytes = contents.as_bytes();
    let key_bytes = key.as_bytes();
    if let Some(pos) = contents.find(key) {
        let after = &contents[pos + key.len()..];
        if let Some(start) = after.find("<string>") {
            let rest = &after[start + 8..];
            if let Some(end) = rest.find("</string>") {
                let id = rest[..end].trim();
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        // binary plists won't match — plutil should have handled those
        let _ = (bytes, key_bytes);
    }
    None
}

/// Get bundle IDs of currently running processes via launchctl
pub fn get_running_bundle_ids() -> HashSet<String> {
    let mut ids = HashSet::new();

    let output = match Command::new("launchctl").arg("list").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return ids,
    };

    for line in output.lines().skip(1) {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let label = parts[2].trim();
            if label.contains('.') && !label.contains(' ') {
                ids.insert(label.to_string());
                for base in strip_all_service_suffixes(label) {
                    ids.insert(base);
                }
            }
        }
    }

    ids
}

/// Strip known service suffixes repeatedly (helper.agent → helper → app).
pub fn strip_service_suffix(label: &str) -> Option<String> {
    strip_all_service_suffixes(label).into_iter().next()
}

fn strip_all_service_suffixes(label: &str) -> Vec<String> {
    const SUFFIXES: &[&str] = &[
        ".helper",
        ".agent",
        ".daemon",
        ".xpc",
        ".service",
        ".launcher",
        ".updater",
        ".GPU",
        ".Renderer",
        ".Plugin",
    ];
    let mut results = Vec::new();
    let mut current = label.to_string();
    loop {
        let mut stripped = false;
        for suffix in SUFFIXES {
            if let Some(base) = current.strip_suffix(suffix) {
                if base.contains('.') {
                    results.push(base.to_string());
                    current = base.to_string();
                    stripped = true;
                    break;
                }
            }
        }
        if !stripped {
            break;
        }
    }
    results
}

/// Known prefixes that should never be treated as orphans (system / package managers).
pub fn is_whitelisted_prefix(bundle_id: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "com.apple.",
        "com.applee.",
        "gov.apple.",
        "homebrew.",
        "sh.brew.",
        "org.homebrew.",
        "com.github.Homebrew",
        "org.freedesktop.",
        "org.mozilla.",
    ];
    PREFIXES.iter().any(|p| bundle_id.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_service_suffixes() {
        assert_eq!(
            strip_service_suffix("com.foo.app.helper"),
            Some("com.foo.app".into())
        );
        let multi = strip_all_service_suffixes("com.foo.app.helper.agent");
        assert!(multi.contains(&"com.foo.app.helper".to_string()));
        assert!(multi.contains(&"com.foo.app".to_string()));
    }

    #[test]
    fn installed_bundle_ids_runs_without_panic() {
        // May Err if Spotlight is off in CI — just must not panic
        let _ = get_installed_bundle_ids();
    }

    #[test]
    fn extracts_from_xml_plist() {
        let xml = r#"<?xml version="1.0"?>
        <dict>
          <key>CFBundleIdentifier</key>
          <string>com.example.app</string>
        </dict>"#;
        assert_eq!(
            extract_bundle_id_from_plist_text(xml),
            Some("com.example.app".into())
        );
    }
}
