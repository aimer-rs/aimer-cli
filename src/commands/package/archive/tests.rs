use std::io::Read;

use super::*;
use crate::commands::package::test_support::args_of;

/// What a reader finds in the central directory of a zip.
#[derive(Debug)]
struct Item {
    name: String,
    method: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    mode: u32,
    offset: usize,
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Read the central directory the way an unzipper does, from the end.
fn read_directory(zip: &[u8]) -> Vec<Item> {
    let end = zip.len() - 22;
    assert_eq!(
        le32(zip, end),
        END_OF_DIRECTORY,
        "no end-of-directory record"
    );
    let count = usize::from(le16(zip, end + 10));
    let mut at = le32(zip, end + 16) as usize;
    let mut items = Vec::new();
    for _ in 0..count {
        assert_eq!(le32(zip, at), CENTRAL_HEADER);
        let name_len = usize::from(le16(zip, at + 28));
        items.push(Item {
            method: le16(zip, at + 10),
            crc: le32(zip, at + 16),
            compressed: le32(zip, at + 20),
            size: le32(zip, at + 24),
            mode: le32(zip, at + 38) >> 16,
            offset: le32(zip, at + 42) as usize,
            name: String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).unwrap(),
        });
        at += 46 + name_len;
    }
    items
}

/// The uncompressed content of an entry, checked against its CRC.
fn content(zip: &[u8], item: &Item) -> Vec<u8> {
    let at = item.offset;
    assert_eq!(le32(zip, at), LOCAL_HEADER);
    let start = at + 30 + usize::from(le16(zip, at + 26)) + usize::from(le16(zip, at + 28));
    let data = &zip[start..start + item.compressed as usize];
    let bytes = match item.method {
        0 => data.to_vec(),
        8 => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(data)
                .read_to_end(&mut out)
                .unwrap();
            out
        }
        other => panic!("method {other}"),
    };
    let mut crc = Crc::new();
    crc.update(&bytes);
    assert_eq!(crc.sum(), item.crc, "{}", item.name);
    assert_eq!(bytes.len(), item.size as usize, "{}", item.name);
    bytes
}

fn app_folder(root: &Path) -> PathBuf {
    let app = root.join("bundle");
    std::fs::create_dir_all(app.join("assets/img")).unwrap();
    std::fs::write(app.join("my_app"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::write(app.join("assets/img/logo.png"), vec![7u8; 100_000]).unwrap();
    std::fs::write(app.join("empty.txt"), b"").unwrap();
    app
}

fn zip_of(root: &Path, app: &Path) -> Vec<u8> {
    let output = root.join("out.zip");
    create_zip(app, "my_app-1.2.3", &output).unwrap();
    std::fs::read(output).unwrap()
}

#[test]
fn every_file_and_folder_is_listed_under_one_top_level_folder() {
    let dir = tempfile::tempdir().unwrap();
    let zip = zip_of(dir.path(), &app_folder(dir.path()));

    let names: Vec<String> = read_directory(&zip).into_iter().map(|i| i.name).collect();

    assert_eq!(
        names,
        [
            "my_app-1.2.3/",
            "my_app-1.2.3/assets/",
            "my_app-1.2.3/assets/img/",
            "my_app-1.2.3/assets/img/logo.png",
            "my_app-1.2.3/empty.txt",
            "my_app-1.2.3/my_app",
        ]
    );
}

#[test]
fn files_round_trip_with_their_content_and_crc() {
    let dir = tempfile::tempdir().unwrap();
    let zip = zip_of(dir.path(), &app_folder(dir.path()));
    let items = read_directory(&zip);

    let by_name = |name: &str| items.iter().find(|i| i.name == name).unwrap();
    assert_eq!(
        content(&zip, by_name("my_app-1.2.3/my_app")),
        b"#!/bin/sh\necho hi\n"
    );
    assert_eq!(
        content(&zip, by_name("my_app-1.2.3/assets/img/logo.png")),
        vec![7u8; 100_000]
    );
    assert_eq!(content(&zip, by_name("my_app-1.2.3/empty.txt")), b"");
}

#[test]
fn compressible_files_are_deflated_and_folders_stored() {
    let dir = tempfile::tempdir().unwrap();
    let zip = zip_of(dir.path(), &app_folder(dir.path()));
    let items = read_directory(&zip);

    let logo = items.iter().find(|i| i.name.ends_with("logo.png")).unwrap();
    assert_eq!(logo.method, 8);
    assert!(logo.compressed < logo.size / 10, "{logo:?}");
    let folder = items.iter().find(|i| i.name == "my_app-1.2.3/").unwrap();
    assert_eq!((folder.method, folder.size), (0, 0));
}

#[cfg(unix)]
#[test]
fn the_executable_bit_survives() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let app = app_folder(dir.path());
    std::fs::set_permissions(app.join("my_app"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(
        app.join("empty.txt"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let zip = zip_of(dir.path(), &app);
    let items = read_directory(&zip);

    let mode = |name: &str| items.iter().find(|i| i.name.ends_with(name)).unwrap().mode;
    assert_eq!(mode("/my_app") & 0o7777, 0o755);
    assert_eq!(mode("empty.txt") & 0o7777, 0o600);
    assert_eq!(mode("/my_app") & 0o170000, 0o100000, "a regular file");
    assert_eq!(mode("assets/") & 0o170000, 0o040000, "a folder");
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_stored_as_a_link_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_folder(dir.path());
    std::os::unix::fs::symlink("my_app", app.join("current")).unwrap();
    let zip = zip_of(dir.path(), &app);
    let items = read_directory(&zip);

    let link = items.iter().find(|i| i.name.ends_with("/current")).unwrap();
    assert_eq!(link.mode & 0o170000, 0o120000);
    assert_eq!(content(&zip, link), b"my_app");
}

#[test]
fn the_archive_is_the_same_every_time_for_the_same_folder() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_folder(dir.path());

    assert_eq!(zip_of(dir.path(), &app), zip_of(dir.path(), &app));
}

#[test]
fn offsets_in_the_directory_point_at_local_headers() {
    let dir = tempfile::tempdir().unwrap();
    let zip = zip_of(dir.path(), &app_folder(dir.path()));

    for item in read_directory(&zip) {
        assert_eq!(le32(&zip, item.offset), LOCAL_HEADER, "{}", item.name);
        let name_len = usize::from(le16(&zip, item.offset + 26));
        let local = &zip[item.offset + 30..item.offset + 30 + name_len];
        assert_eq!(local, item.name.as_bytes());
    }
}

#[test]
fn a_missing_folder_is_an_error() {
    let dir = tempfile::tempdir().unwrap();

    let err = create_zip(&dir.path().join("nope"), "top", &dir.path().join("o.zip"));

    assert!(err.is_err());
}

#[test]
fn the_msdos_date_and_time_are_decoded_from_the_epoch() {
    // 2024-02-29 13:45:30 UTC, a leap day.
    let when = UNIX_EPOCH + std::time::Duration::from_secs(1_709_214_330);
    let (time, date) = dos_datetime(when);

    assert_eq!(
        ((date >> 9) + 1980, (date >> 5) & 0xF, date & 0x1F),
        (2024, 2, 29)
    );
    assert_eq!(
        (time >> 11, (time >> 5) & 0x3F, (time & 0x1F) * 2),
        (13, 45, 30)
    );
}

#[test]
fn dates_before_1980_are_clamped_to_the_start_of_the_format() {
    let (time, date) = dos_datetime(UNIX_EPOCH);

    assert_eq!(
        (time, (date >> 9) + 1980, (date >> 5) & 0xF, date & 0x1F),
        (0, 1980, 1, 1)
    );
}

#[test]
fn tar_writes_the_archive_from_the_folder_inside_the_parent() {
    let cmd = tar_command(Path::new("/w"), "My App.app", Path::new("/d/out.tar.gz"));

    assert_eq!(cmd.get_program(), "tar");
    assert_eq!(
        args_of(&cmd),
        ["-czf", "/d/out.tar.gz", "-C", "/w", "My App.app"]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(
        envs.iter()
            .any(|(k, v)| *k == "COPYFILE_DISABLE" && *v == Some("1".as_ref()))
    );
}

#[test]
fn copy_dir_copies_nested_files_and_keeps_the_executable_bit() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_folder(dir.path());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(app.join("my_app"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    copy_dir(&app, &dir.path().join("copy/deeper")).unwrap();

    assert_eq!(
        std::fs::read(dir.path().join("copy/deeper/assets/img/logo.png"))
            .unwrap()
            .len(),
        100_000
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("copy/deeper/my_app"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}
