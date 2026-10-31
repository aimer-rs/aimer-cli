[← Back to README](../README.md)

# Command Reference

- [`aimer create`](#aimer-create)
- [`aimer init`](#aimer-init)
- [`aimer run`](#aimer-run)
- [`aimer build`](#aimer-build)
- [`aimer assemble`](#aimer-assemble)
- [`aimer package`](#aimer-package)
- [`aimer doctor`](#aimer-doctor)
- [`aimer clean`](#aimer-clean)
- [`aimer migrate`](#aimer-migrate)
- [`aimer completions`](#aimer-completions)
- [`aimer mcp`](#aimer-mcp)
- [`aimer skill`](#aimer-skill)
- [`aimer sessions`](#aimer-sessions)

---

## `aimer create`

Scaffolds a new Aimer project with target configurations, templates, and platform build directories.

```bash
aimer create <PROJECT_NAME>
```

---

## `aimer init`

Turns an existing Cargo project into an Aimer project. It asks the same questions as `aimer create` and generates the
same `Aimer.toml` and `builds/<platform>/` scaffolds, but works in place: your sources and README are left alone, and
`Cargo.toml` (an Aimer-capable `[lib]` and the `aimer` dependency) and `.gitignore` are only extended.
It refuses to run on a project that already has an `Aimer.toml` (use `aimer migrate`) or to replace an existing
`builds/<platform>/` folder.

```bash
aimer init [PATH]
```

**Arguments:**

- `[PATH]`: Cargo project directory (defaults to the current directory).

Your entry point still has to start the app, as in the `aimer create` template (`#[aimer::main]` and
`AimerApp::start(...)`).

---

## `aimer run`

Builds, packages, and executes the project with an interactive developer console. By default the console is rendered
inline: finished stages scroll into your terminal history while the active stage and a hotkey bar stay at the bottom.
Use `--tui` for the full-screen console, or `--no-tui` for plain logs. When stdin or stdout is not a terminal (CI, an
IDE run window, a pipe), `aimer run` prints plain logs automatically.

```bash
aimer run [OPTIONS]
```

**Options:**

| Option                | Flag | Description                                                                                        |
|:----------------------|:-----|:---------------------------------------------------------------------------------------------------|
| `--target`            | `-t` | Specify the target platform directly (`macos`, `windows`, `linux`, `android`, `ios`, `web`)        |
| `--device`            | `-d` | Target a specific device ID without prompting                                                      |
| `--package`           | `-p` | Select an Aimer package in a Cargo workspace                                                       |
| `--release`           | `-r` | Build and package with release optimizations                                                       |
| `--tui`               |      | Use the full-screen TUI console instead of the default inline console (conflicts with `--no-tui`)  |
| `--no-tui`            |      | Disable the interactive console and stream plain logs to stdout/stderr (ideal for CI and IDEs)     |
| `--no-mcp`            |      | Disable the background private MCP control session                                                 |
| `-Z <FEATURE>`        |      | Enable nightly-only unstable features (`hot-reload`)                                               |
| `--verbose-widget-ir` |      | Print every Widget IR stage after native materialization (requires `-Z`)                           |
| `--verbose`           |      | Enable debug-level CLI logging                                                                     |

See [Interactive Developer Console](CONSOLE.md) for keyboard shortcuts, and
[Hot Reload & Unstable Features](HOT_RELOAD.md) for `-Z hot-reload`.

---

## `aimer build`

Compiles the Rust crate for the specified target without assembling a full platform package or launching the app.

```bash
aimer build [OPTIONS]
```

**Options:**

| Option      | Flag | Description                                                                           |
|:------------|:-----|:--------------------------------------------------------------------------------------|
| `--target`  | `-t` | Target platform to compile for (defaults to `[build].default_target` in `Aimer.toml`) |
| `--package` | `-p` | Select an Aimer package in a Cargo workspace                                        |
| `--release` | `-r` | Compile with release optimizations                                                    |

---

## `aimer assemble`

Assembles the distributable platform bundle (e.g. `.app`, `.apk`, `.ipa`, or static web directory).

```bash
aimer assemble [OPTIONS] <PLATFORM>
```

**Arguments:**

- `<PLATFORM>`: Target platform to bundle (`macos`, `windows`, `linux`, `android`, `ios`, `web`).

**Options:**

- `-r, --release`: Assemble in release mode for production distribution.
- `-p, --package <NAME>`: Select an Aimer package in a Cargo workspace.

Run, build, and assemble discover Aimer projects from `Aimer.toml` or `aimer.toml` files in the Cargo workspace. From
the workspace root, a single Aimer package is selected automatically. If there are multiple, use `-p` or `--package`.

**Examples:**

```bash
# Assemble release macOS .app bundle
aimer assemble macos --release

# Assemble release Android .apk
aimer assemble android --release

# Assemble Web distribution files
aimer assemble web --release

# Select one Aimer package in a workspace
aimer assemble android -p moewmoew
```

---

## `aimer package`

Builds the app and wraps it in a distributable package: a `.dmg` disk image or a `.pkg` installer on macOS, an `.ipa` on
iOS, and a signed `.aab` or `.apk` on Android. The Apple platforms need a macOS host; Android works on any host.

```bash
aimer package [OPTIONS] <PLATFORM>
```

**Arguments:**

- `<PLATFORM>`: Target platform to package: `macos`, `ios` or `android`.

**Options:**

- `-f, --format <FORMAT>`: Package format: `dmg` or `pkg` on macOS (default `dmg`), `ipa` on iOS (default), `aab` or
  `apk` on Android (default `aab`).
- `-r, --release`: Package a release build. This is the default, so the flag only states it explicitly.
- `--debug`: Package a debug build instead (macOS and iOS; Android packages are always release builds). Cannot be
  combined with `--release`.
- `-p, --package <NAME>`: Select an Aimer package in a Cargo workspace.
- `--sign`: macOS: code-sign the app and the package (see [Signing and notarization](#signing-and-notarization)).
- `--notarize`: macOS: sign, then have Apple notarize the package and staple the ticket. Implies `--sign`.
- `--upload`: iOS: upload the build to App Store Connect (see [iOS and the App Store](#ios-and-the-app-store)).

Unlike `build` and `assemble`, `package` builds in release mode unless `--debug` is given, since a package is meant for
distribution.

The package is written to `builds/<macos|ios|android>/dist/<release|debug>/<name>-<version>.<ext>`, where `<name>` is the application
name and `<version>` comes from `Aimer.toml` (omitted when empty).

- **`dmg`**: a compressed disk image holding the `.app` next to an `Applications` shortcut. Opening it shows an install
  window ("To install, drag *App* to Applications") whose background gradient is tinted with the colors of the app's
  icon. The image is made with `diskutil image` (macOS releases without it fall back to `hdiutil` and a plain image).
  Finder records the window layout, so styling needs a logged-in macOS session and, the first time, permission for the
  terminal to control Finder. When Finder is unavailable, or a volume with the app's name is already mounted, a warning
  is printed and a plain image is created instead.
- **`pkg`**: a flat installer that places the `.app` in `/Applications` (`pkgbuild`). The package identifier and version
  are read from the app's `Info.plist`.

Without `--sign`, packages are not code-signed or notarized, so other Macs show a Gatekeeper warning when opening them.

**Examples:**

```bash
# Release .dmg (the default format and the default mode)
aimer package macos

# Release .pkg installer
aimer package macos -f pkg

# Signed, notarized and stapled .dmg
aimer package macos --notarize

# Debug build, e.g. to try the installer layout quickly
aimer package macos --debug
```

### Android and Google Play

`aimer package android` builds the Rust library, then has Gradle build a **signed release** package: an **`.aab`** (the
Android App Bundle that Google Play requires for new apps, the default) or an **`.apk`** (`-f apk`, for installing
directly or for other stores).

```bash
aimer package android            # builds/android/dist/release/<name>-<version>.aab
aimer package android -f apk     # ... or an .apk
```

The generated project signs release builds with the debug key so they can run locally. `aimer package` replaces that with
your own key at build time (a Gradle init script), so the project does not change and existing projects work as they
are. Debug builds are not packaged; `aimer assemble android` makes those.

You need a keystore. `keytool -genkeypair -v -keystore upload.jks -alias upload -keyalg RSA -keysize 2048 -validity
10000` makes one. Names and paths go in `Aimer.toml`; **passwords are read from the environment only**:

```toml
[android]
version_code = 12                 # optional, see below

[android.signing]
keystore = "keys/upload.jks"      # relative to the project root
key_alias = "upload"
```

| Variable                          | Overrides / provides                                                   |
| --------------------------------- | ---------------------------------------------------------------------- |
| `AIMER_ANDROID_KEYSTORE`          | `keystore`                                                             |
| `AIMER_ANDROID_KEY_ALIAS`         | `key_alias`                                                            |
| `AIMER_ANDROID_KEYSTORE_PASSWORD` | the keystore password (required)                                       |
| `AIMER_ANDROID_KEY_PASSWORD`      | the key password; defaults to the keystore password                    |
| `AIMER_ANDROID_VERSION_CODE`      | `version_code`                                                         |

- **Checked first.** Before the build, the keystore must exist and open with the password and alias, so a mistake fails in
  seconds.
- **Version.** `versionName` is `version` from `Aimer.toml`. `versionCode` is `version_code` if set, otherwise
  `major * 1,000,000 + minor * 1,000 + patch` (`1.2.3` is `1002003`), which grows with every release of a normal version
  number. Google Play rejects an upload whose `versionCode` is not higher than the last one, so on CI set
  `AIMER_ANDROID_VERSION_CODE` to a build number.
- **Google Play signing.** For an `.aab`, sign with your **upload key**. With Play App Signing (the default for new apps),
  Google re-signs the app with the app signing key it holds. Keep the upload keystore safe: losing it means asking Google
  to reset the upload key.
- **ABI.** The library is built for `arm64-v8a`, as in `aimer assemble android`.
- **Uploading** the `.aab` to Play Console is not part of `aimer package` yet; upload it in the Play Console or with
  another tool.

### iOS and the App Store

`aimer package ios` follows Apple's own route, the one Xcode's Organizer uses: it builds the Rust library for devices,
**archives** the Xcode project, then **exports** the archive for a distribution method, which re-signs it and writes the
`.ipa`. Assets are copied into the archived app and the version from `Aimer.toml` becomes `CFBundleShortVersionString`.
iOS builds are always signed, so `--sign` is not needed.

```bash
# Archive and export an .ipa for App Store Connect
aimer package ios

# ... and upload it
aimer package ios --upload
```

You need an Apple Developer team. Names go in `Aimer.toml`:

```toml
[ios.signing]
team_id = "ABCDE12345"
method = "app-store-connect"      # or release-testing, enterprise, debugging
allow_provisioning_updates = true # see below
```

| Variable                                                       | Overrides                    |
| -------------------------------------------------------------- | ---------------------------- |
| `AIMER_IOS_TEAM_ID`                                            | `team_id`                    |
| `AIMER_IOS_EXPORT_METHOD`                                      | `method`                     |
| `AIMER_IOS_ALLOW_PROVISIONING_UPDATES`                         | `allow_provisioning_updates` |
| `AIMER_ASC_KEY`, `AIMER_ASC_KEY_ID`, `AIMER_ASC_ISSUER`        | an App Store Connect API key |

- **Signing is automatic.** Xcode picks the certificates and provisioning profiles. By default `xcodebuild` may not
  change your developer account, so a first build can stop with "No profiles for '*bundle id*' were found". Set
  `allow_provisioning_updates = true` to let it create the App ID and profiles (`-allowProvisioningUpdates`).
- **Credentials.** When Xcode is signed in to the developer account (Xcode, Settings, Accounts) nothing else is needed.
  On CI use an App Store Connect API key: the three `AIMER_ASC_*` variables, all or none. The key file is a `.p8` from
  *Users and Access, Integrations* in App Store Connect.
- **The bundle ID** is the `group` of `Aimer.toml`, and the app has to exist in App Store Connect before an upload.
- **`--upload`** runs a second export whose destination is App Store Connect. It needs the `app-store-connect` method
  and a release build. Xcode manages the build number on upload, so a repeated upload of the same version does not
  clash. The `.ipa` stays in `builds/ios/dist/` either way.
- **The other methods** (`release-testing`, `enterprise`, `debugging`) only write the `.ipa`.

### Signing and notarization

`--sign` and `--notarize` need an Apple Developer account and, once per machine:

- a **Developer ID Application** certificate in the keychain (signs the app and the `.dmg`);
- for `.pkg`, also a **Developer ID Installer** certificate;
- for `--notarize`, notary credentials: a profile saved with `xcrun notarytool store-credentials <profile>`, or an App
  Store Connect API key.

Names and paths go in `Aimer.toml`; secrets never do:

```toml
[macos.signing]
identity = "Developer ID Application: Your Name (TEAMID)"
installer_identity = "Developer ID Installer: Your Name (TEAMID)" # .pkg only
entitlements = "builds/macos/App.entitlements"                     # optional
notary_profile = "aimer-notary"
```

Environment variables override the manifest, which suits CI:

| Variable                                                          | Overrides            |
| ----------------------------------------------------------------- | -------------------- |
| `AIMER_MACOS_SIGN_IDENTITY`                                       | `identity`           |
| `AIMER_MACOS_INSTALLER_IDENTITY`                                  | `installer_identity` |
| `AIMER_MACOS_ENTITLEMENTS`                                        | `entitlements`       |
| `AIMER_NOTARY_PROFILE`                                            | `notary_profile`     |
| `AIMER_NOTARY_KEY`, `AIMER_NOTARY_KEY_ID`, `AIMER_NOTARY_ISSUER`  | an API key, used instead of a profile when all three are set |

Before building, the command checks that the identities are in the keychain, that the entitlements file exists and that
the notary credentials are accepted, so a mistake fails in seconds instead of after the build or the notarization wait.

The steps, in order: sign the `.app` with the hardened runtime and verify it; build the `.dmg` (and sign it) or the
`.pkg` (signed by `pkgbuild`); with `--notarize`, submit it with `xcrun notarytool submit --wait`, then
`xcrun stapler staple`. Notarizing the package covers the app inside it; the app itself is not stapled separately.

---

## `aimer doctor`

Diagnoses your development environment, reporting installed compilers, platform SDK tools, and cross-compilation target
triples.

```bash
aimer doctor
```

**What it verifies:**

- Rust compiler (`rustc`) & Cargo
- Web bundler (`trunk`)
- Xcode Command Line Tools (`xcrun`)
- Android SDK & Bridge (`adb`, `gradle`)
- Host linkers (`cc`, `pkg-config`, MSVC `link`)
- Required `rustup` target triples for all supported platforms, providing copy-paste commands for missing targets

---

## `aimer clean`

Removes build artifacts, generated files, intermediate libraries, and runs `cargo clean`.

```bash
aimer clean
```

Cleans:

- `builds/*/build/`
- `builds/android/app/build/` and `jniLibs/`
- `builds/web/pkg/` and `node_modules/`
- Cargo `target/` directory

---

## `aimer migrate`

Upgrades existing platform build scaffolds (`builds/<platform>/`) to the latest templates bundled with the current CLI
version while preserving your project settings and code.

```bash
aimer migrate <TARGET>
```

**Arguments:**

- `<TARGET>`: Platform to migrate (`macos`, `windows`, `linux`, `android`, `ios`, `web`, or `all`).

---

## `aimer completions`

Generates dynamic, self-updating shell completion scripts.

```bash
aimer completions <SHELL> [--install]
```

**Arguments:**

- `<SHELL>`: Shell type (`bash`, `zsh`, `fish`, `elvish`, `powershell`).

**Options:**

- `--install`: Writes the completion file into the conventional user completion directory and prints activation hints.

---

## `aimer mcp`

> Requires a build with the `mcp` feature (or `bundled-proc-macro` / `legacy-lsp`, which imply it). See [Installation](INSTALLATION.md).

Runs an MCP (Model Context Protocol) stdio server for Rust project tools and crate documentation. When the selected
project is an Aimer project with a live session, it also exposes session controls, logs, and resource monitoring. The
adapter discovers the nearest `Aimer.toml` or `Cargo.toml` from its working directory. Use `--attach` to select a
specific live Aimer session when the project has more than one.

```bash
# Install MCP into AI agents:
aimer mcp --install <claude|cursor|codex> [--global] [--project <PATH>]

# Run the MCP stdio adapter:
aimer mcp [--project <PATH>] [--attach <SESSION_ID>]
```

See [Model Context Protocol (MCP) Integration](MCP.md) for supported agents and the full tool list.

---

## `aimer skill`

Install or update Aimer's Rust workflow skill in the selected agent's user-level skills directory.

```bash
aimer skill --install <codex|claude|cursor> [<AGENT> ...]
```

For example, `aimer skill --install claude,codex` installs it for Claude Code and Codex. The command installs
`aimer-rust/SKILL.md`; its MCP tool list is generated from the server's `tools/list` response by
`scripts/update_aimer_rust_skill.py`.

---

## `aimer sessions`

Lists all live Aimer run sessions currently active on the machine.

```bash
aimer sessions [--project <PATH>] [--json]
```

**Options:**

- `--project <PATH>`: Restrict session discovery to a specific project directory.
- `--json`: Output session details in machine-readable JSON.
