//! The `[permissions]` table of `Aimer.toml`.
//!
//! One portable table declares what the app may access. `aimer assemble`
//! translates it into `Info.plist` keys (iOS and macOS) and `<uses-permission>`
//! elements (Android), so the permissions are declared once.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The portable `[permissions]` table.
///
/// A string is the reason shown to the user: Apple requires it for the
/// permission prompt, Android has no equivalent and ignores it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Camera access (`NSCameraUsageDescription`, `CAMERA`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// Microphone access (`NSMicrophoneUsageDescription`, `RECORD_AUDIO`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
    /// Location while the app is in use
    /// (`NSLocationWhenInUseUsageDescription`, `ACCESS_FINE_LOCATION` and
    /// `ACCESS_COARSE_LOCATION`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The photo library (`NSPhotoLibraryUsageDescription`,
    /// `READ_MEDIA_IMAGES` and, up to Android 12, `READ_EXTERNAL_STORAGE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photos: Option<String>,
    /// The address book (`NSContactsUsageDescription`, `READ_CONTACTS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contacts: Option<String>,
    /// Network access. Android only (`INTERNET`); Apple platforms need no
    /// declaration for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internet: Option<bool>,
}

/// An Android `<uses-permission>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidUse {
    /// The permission name, e.g. `android.permission.CAMERA`.
    pub name: String,
    /// The `android:maxSdkVersion` attribute, when the permission only
    /// applies up to an API level.
    pub max_sdk_version: Option<u32>,
}

impl AndroidUse {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            max_sdk_version: None,
        }
    }
}

/// The `[android.permissions]` table: permissions with no portable key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidPermissionsMeta {
    /// Full permission names, e.g. `android.permission.VIBRATE`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
}

impl Permissions {
    /// Whether no portable permission is declared.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// The `Info.plist` entries (key, string value) for the Apple platforms.
    ///
    /// `extra` is the platform's own `[ios.permissions]` or
    /// `[macos.permissions]` table; it wins over the portable keys.
    pub fn apple_entries(&self, extra: &BTreeMap<String, String>) -> Vec<(String, String)> {
        let portable = [
            ("NSCameraUsageDescription", &self.camera),
            ("NSMicrophoneUsageDescription", &self.microphone),
            ("NSLocationWhenInUseUsageDescription", &self.location),
            ("NSPhotoLibraryUsageDescription", &self.photos),
            ("NSContactsUsageDescription", &self.contacts),
        ];
        let mut entries: BTreeMap<String, String> = portable
            .into_iter()
            .filter_map(|(key, reason)| Some((key.to_string(), reason.clone()?)))
            .collect();
        entries.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        entries.into_iter().collect()
    }

    /// The Android `<uses-permission>` elements, without duplicates.
    ///
    /// `extra` is the `[android.permissions]` table.
    pub fn android_uses(&self, extra: Option<&AndroidPermissionsMeta>) -> Vec<AndroidUse> {
        let mut uses: Vec<AndroidUse> = Vec::new();
        if self.internet == Some(true) {
            uses.push(AndroidUse::new("android.permission.INTERNET"));
        }
        if self.camera.is_some() {
            uses.push(AndroidUse::new("android.permission.CAMERA"));
        }
        if self.microphone.is_some() {
            uses.push(AndroidUse::new("android.permission.RECORD_AUDIO"));
        }
        if self.location.is_some() {
            uses.push(AndroidUse::new("android.permission.ACCESS_FINE_LOCATION"));
            uses.push(AndroidUse::new("android.permission.ACCESS_COARSE_LOCATION"));
        }
        if self.photos.is_some() {
            uses.push(AndroidUse::new("android.permission.READ_MEDIA_IMAGES"));
            uses.push(AndroidUse {
                name: "android.permission.READ_EXTERNAL_STORAGE".to_string(),
                max_sdk_version: Some(32),
            });
        }
        if self.contacts.is_some() {
            uses.push(AndroidUse::new("android.permission.READ_CONTACTS"));
        }
        for name in extra.map(|e| e.uses.as_slice()).unwrap_or_default() {
            if !uses.iter().any(|u| &u.name == name) {
                uses.push(AndroidUse::new(name));
            }
        }
        uses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AimerManifest;

    fn parse(permissions: &str) -> AimerManifest {
        toml::from_str(&format!("[package]\nname = \"App\"\n{permissions}")).unwrap()
    }

    #[test]
    fn the_portable_table_is_read_from_the_manifest() {
        let manifest = parse(
            "[permissions]\ncamera = \"Scan codes\"\ninternet = true\n\
             [ios.permissions]\nNSFaceIDUsageDescription = \"Unlock\"\n\
             [android.permissions]\nuses = [\"android.permission.VIBRATE\"]\n",
        );

        assert_eq!(manifest.permissions.camera.as_deref(), Some("Scan codes"));
        assert_eq!(manifest.permissions.internet, Some(true));
        assert_eq!(
            manifest.ios.unwrap().permissions["NSFaceIDUsageDescription"],
            "Unlock"
        );
        assert_eq!(
            manifest.android.unwrap().permissions.unwrap().uses,
            ["android.permission.VIBRATE"]
        );
    }

    #[test]
    fn manifests_without_permissions_still_load_and_are_not_written_back() {
        let manifest = parse("");
        assert!(manifest.permissions.is_empty());

        let written = toml::to_string(&AimerManifest::new("App", "1.0.0", "", "", "")).unwrap();
        assert!(!written.contains("permissions"));
    }

    #[test]
    fn apple_entries_map_the_portable_keys_and_let_the_platform_table_win() {
        let permissions = Permissions {
            camera: Some("Scan".into()),
            location: Some("Nearby".into()),
            internet: Some(true),
            ..Permissions::default()
        };
        let extra = BTreeMap::from([
            ("NSCameraUsageDescription".to_string(), "Custom".to_string()),
            ("NSFaceIDUsageDescription".to_string(), "Unlock".to_string()),
        ]);

        assert_eq!(
            permissions.apple_entries(&extra),
            [
                ("NSCameraUsageDescription".into(), "Custom".into()),
                ("NSFaceIDUsageDescription".into(), "Unlock".into()),
                (
                    "NSLocationWhenInUseUsageDescription".into(),
                    "Nearby".into()
                ),
            ]
        );
    }

    #[test]
    fn android_uses_map_the_portable_keys_without_duplicates() {
        let permissions = Permissions {
            camera: Some("Scan".into()),
            photos: Some("Pick".into()),
            internet: Some(true),
            ..Permissions::default()
        };
        let extra = AndroidPermissionsMeta {
            uses: vec![
                "android.permission.CAMERA".into(),
                "android.permission.VIBRATE".into(),
            ],
        };

        let uses = permissions.android_uses(Some(&extra));
        let names: Vec<&str> = uses.iter().map(|u| u.name.as_str()).collect();

        assert_eq!(
            names,
            [
                "android.permission.INTERNET",
                "android.permission.CAMERA",
                "android.permission.READ_MEDIA_IMAGES",
                "android.permission.READ_EXTERNAL_STORAGE",
                "android.permission.VIBRATE",
            ]
        );
        assert_eq!(uses[3].max_sdk_version, Some(32));
    }

    #[test]
    fn internet_false_declares_nothing() {
        let permissions = Permissions {
            internet: Some(false),
            ..Permissions::default()
        };

        assert!(permissions.android_uses(None).is_empty());
    }
}
