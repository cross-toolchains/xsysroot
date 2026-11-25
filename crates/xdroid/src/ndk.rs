//! Android NDK release metadata and official Google CDN URL construction.

use std::env;

pub const DEFAULT_NDK_VERSION: &str = "r27c";
pub const DEFAULT_API_LEVEL: u32 = 24;

/// Known Android NDK release tags hosted on `dl.google.com`.
pub const KNOWN_RELEASES: &[&str] = &["r28", "r27c", "r26d"];

/// Target Android triples for which compiler shims are generated.
pub const ANDROID_TRIPLES: &[&str] = &[
    "aarch64-linux-android",
    "armv7a-linux-androideabi",
    "x86_64-linux-android",
    "i686-linux-android",
];

/// Returns the Google NDK host platform tag for the current OS (`darwin`, `linux`, `windows`).
pub fn host_ndk_os() -> Result<&'static str, String> {
    match env::consts::OS {
        "macos" => Ok("darwin"),
        "linux" => Ok("linux"),
        "windows" => Ok("windows"),
        other => Err(format!("unsupported host OS for Android NDK: `{other}`")),
    }
}

/// Builds the official unauthenticated `dl.google.com` download URL for `ndk_version`.
pub fn ndk_download_url(ndk_version: &str, host_os: &str) -> String {
    format!("https://dl.google.com/android/repository/android-ndk-{ndk_version}-{host_os}.zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_official_google_ndk_urls() {
        assert_eq!(
            ndk_download_url("r27c", "darwin"),
            "https://dl.google.com/android/repository/android-ndk-r27c-darwin.zip"
        );
        assert_eq!(
            ndk_download_url("r27c", "linux"),
            "https://dl.google.com/android/repository/android-ndk-r27c-linux.zip"
        );
    }
}
