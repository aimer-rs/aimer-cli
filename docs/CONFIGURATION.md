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

# Optional hot-reload guest configuration (unstable / nightly)
[build.hot_reload]
package = "my_aimer_app"
program = "my_aimer_app::AppProgram"
limits = "my_aimer_app::APP_LIMITS"
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
