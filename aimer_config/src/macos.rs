//! The `[macos]` table of `Aimer.toml`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Settings that only apply to the macOS target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacosMeta {
    /// The `[macos.signing]` table used by `aimer package --sign`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<MacosSigningMeta>,
    /// The `[macos.permissions]` table: raw `Info.plist` keys and their string
    /// values, for what the portable `[permissions]` table does not cover.
    /// They win over the portable keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, String>,
}

/// Code signing and notarization settings for `aimer package`.
///
/// These are names and paths only. Secrets (certificates, App Store Connect
/// keys) stay in the keychain or in environment variables, never in the
/// manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacosSigningMeta {
    /// Name or SHA-1 of the "Developer ID Application" identity that signs the
    /// app and the `.dmg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Name or SHA-1 of the "Developer ID Installer" identity that signs a
    /// `.pkg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer_identity: Option<String>,
    /// Entitlements plist applied when signing the app, relative to the
    /// project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entitlements: Option<String>,
    /// `notarytool` keychain profile (`xcrun notarytool store-credentials`)
    /// used by `aimer package --notarize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notary_profile: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::AimerManifest;

    #[test]
    fn signing_table_is_read_from_the_manifest() {
        let manifest: AimerManifest = toml::from_str(
            r#"
            [package]
            name = "App"

            [macos.signing]
            identity = "Developer ID Application: Me (TEAM)"
            notary_profile = "aimer-notary"
            "#,
        )
        .unwrap();

        let signing = manifest.macos.unwrap().signing.unwrap();
        assert_eq!(
            signing.identity.as_deref(),
            Some("Developer ID Application: Me (TEAM)")
        );
        assert_eq!(signing.notary_profile.as_deref(), Some("aimer-notary"));
        assert_eq!(signing.installer_identity, None);
        assert_eq!(signing.entitlements, None);
    }

    #[test]
    fn manifests_without_a_macos_table_still_load() {
        let manifest: AimerManifest = toml::from_str("[package]\nname = \"App\"\n").unwrap();

        assert_eq!(manifest.macos, None);
    }

    #[test]
    fn an_unset_macos_table_is_not_written_back() {
        let manifest = AimerManifest::new("App", "1.0.0", "", "", "");

        let text = toml::to_string(&manifest).unwrap();

        assert!(!text.contains("macos"), "{text}");
    }
}
