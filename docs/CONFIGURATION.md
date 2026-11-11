[← Back to README](../README.md)

# Project Configuration (`Aimer.toml`)

The `Aimer.toml` manifest file resides at the root of every Aimer project and defines application metadata, build
preferences, and asset bundling.

```toml
[package]
# Application name: the bundle is shipped under it (e.g. `My Aimer App.app`)
# and it is the label shown under the launcher icon. It may contain spaces.
# The Cargo package in `Cargo.toml` keeps the project name.
name = "My Aimer App"
version = "0.1.0"
description = "A cross-platform GUI app built with Aimer"
author = "Your Name <you@example.com>"
group = "com.example.app"

[build]
# Default platform used by `aimer build` when no --target is specified
default_target = "macos"

# Files relative to the project root to bundle across all platforms,
# declared as `name = "path"`
[assets]
icon = "assets/icon.png"
data = "assets/data.json"

[fonts]
inter = "assets/fonts/Inter-Regular.ttf"

# What the app may access, written into Info.plist and AndroidManifest.xml on assemble.
# A string is the reason shown in the iOS and macOS permission prompt.
[permissions]
camera = "Scan QR codes to sign in"
internet = true

# Optional code signing for `aimer package --sign` (macOS), see docs/COMMANDS.md
[macos.signing]
identity = "Developer ID Application: Your Name (TEAMID)"
notary_profile = "aimer-notary"

# Optional signing and App Store distribution for `aimer package ios`, see docs/COMMANDS.md
[ios.signing]
team_id = "ABCDE12345"

# Optional signing for `aimer package android`; passwords come from the environment
[android.signing]
keystore = "keys/upload.jks"
key_alias = "upload"
```

---

## Assets Management

To bundle assets (images, fonts, JSON configuration) into your application across all supported targets:

1. Place your asset files inside your project directory (e.g. `assets/logo.png`).
2. Declare the paths under `[assets]` (and fonts under `[fonts]`) in `Aimer.toml` as `name = "path"` entries:

```toml
[assets]
image_1 = "assets/logo.png"
image_2 = "assets/banner.png"

[fonts]
font_1 = "assets/fonts/Inter-Regular.ttf"
```

When building or assembling for any target (macOS, iOS, Android, Desktop, Web), `aimer` automatically copies these files
into the native platform asset directory (e.g., Android `assets/`, macOS `.app/Contents/Resources/`, iOS bundle
resources, web dist). The asset path declared in `Aimer.toml` serves as the runtime lookup key in your Rust code.

---

## Permissions

Declare what the app may access once, under `[permissions]`. Every time the macOS, iOS or Android app is assembled
(`aimer assemble`, `aimer run`, `aimer package`), `aimer` writes the matching entries into `Info.plist` and
`AndroidManifest.xml`.

```toml
[permissions]
camera = "Scan QR codes to sign in"
microphone = "Record voice notes"
location = "Show nearby stores"
photos = "Pick a profile picture"
contacts = "Invite your friends"
internet = true

# Keys with no portable name
[ios.permissions]        # also [macos.permissions]
NSFaceIDUsageDescription = "Unlock the app"

[android.permissions]
uses = ["android.permission.VIBRATE"]
```

| Key          | `Info.plist` (iOS and macOS)           | `AndroidManifest.xml`                                          |
|--------------|----------------------------------------|----------------------------------------------------------------|
| `camera`     | `NSCameraUsageDescription`             | `CAMERA`                                                       |
| `microphone` | `NSMicrophoneUsageDescription`         | `RECORD_AUDIO`                                                 |
| `location`   | `NSLocationWhenInUseUsageDescription`  | `ACCESS_FINE_LOCATION`, `ACCESS_COARSE_LOCATION`               |
| `photos`     | `NSPhotoLibraryUsageDescription`       | `READ_MEDIA_IMAGES`, `READ_EXTERNAL_STORAGE` (up to API 32)    |
| `contacts`   | `NSContactsUsageDescription`           | `READ_CONTACTS`                                                |
| `internet`   | nothing                                | `INTERNET` (when `true`)                                       |

The string is the reason Apple shows (and requires) in the permission prompt; Android has no equivalent and ignores it.
`[ios.permissions]` and `[macos.permissions]` take raw `Info.plist` keys with string values and win over the portable
keys. `[android.permissions] uses` lists full permission names. Web and desktop targets ignore all of these.

### How the files are kept safe

`Info.plist` and `AndroidManifest.xml` stay your files. `aimer` never regenerates them; it only maintains one block,
between two marker comments, and rebuilds it from `Aimer.toml` on every assemble:

```xml
<!-- aimer:permissions:start: generated from [permissions] in Aimer.toml, edits between the markers are overwritten -->
<key>NSCameraUsageDescription</key>
<string>Scan QR codes to sign in</string>
<!-- aimer:permissions:end -->
```

- **Outside the markers is yours.** Nothing there is rewritten or reordered.
- **Your entry has the same value:** it is left alone and nothing is written for it.
- **Your `Info.plist` entry has another value:** `Aimer.toml` wins. Your entry is turned into a comment
  (`<!-- aimer:overridden by [permissions] in Aimer.toml: ... -->`), never deleted, and the build prints which key
  changed and from what to what.
- **An Android permission you already declared** is not declared a second time.
- **Removing a permission from `Aimer.toml`** removes it from the block on the next assemble. An entry that was
  commented out earlier stays a comment, so you can restore it by hand.
- A file that does not change is not rewritten, and a missing `Info.plist` or manifest is skipped with a warning.
- A start marker without its end marker (or the reverse) stops the sync with an error instead of guessing.
