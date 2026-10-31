//! The `[ios]` table of `Aimer.toml`.

use serde::{Deserialize, Serialize};

/// Settings that only apply to the iOS target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IosMeta {
    /// The `[ios.signing]` table used by `aimer package ios`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<IosSigningMeta>,
}

/// Signing and distribution settings for `aimer package ios`.
///
/// Names and switches only. The App Store Connect API key, the one secret
/// involved, is read from environment variables and never stored here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IosSigningMeta {
    /// The Apple Developer team ID that signs the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// How the archive is exported: `app-store-connect` (the default),
    /// `release-testing`, `enterprise` or `debugging`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Let `xcodebuild` create and update App IDs, certificates and
    /// provisioning profiles in the developer account. Off by default, since it
    /// changes the account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_provisioning_updates: Option<bool>,
}

#[cfg(test)]
mod tests {
    use crate::config::AimerManifest;

    #[test]
    fn signing_table_is_read_from_the_manifest() {
        let manifest: AimerManifest = toml::from_str(
            r#"
            [package]
            name = "App"

            [ios.signing]
            team_id = "ABCDE12345"
            allow_provisioning_updates = true
            "#,
        )
        .unwrap();

        let signing = manifest.ios.unwrap().signing.unwrap();
        assert_eq!(signing.team_id.as_deref(), Some("ABCDE12345"));
        assert_eq!(signing.allow_provisioning_updates, Some(true));
        assert_eq!(signing.method, None);
    }

    #[test]
    fn manifests_without_an_ios_table_still_load() {
        let manifest: AimerManifest = toml::from_str("[package]\nname = \"App\"\n").unwrap();

        assert_eq!(manifest.ios, None);
    }

    #[test]
    fn an_unset_ios_table_is_not_written_back() {
        let manifest = AimerManifest::new("App", "1.0.0", "", "", "");

        assert!(!toml::to_string(&manifest).unwrap().contains("ios"));
    }
}
