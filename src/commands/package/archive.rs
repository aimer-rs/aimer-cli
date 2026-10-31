//! Archives of an app folder: the `.zip` writer and the `tar` command shared by
//! the macOS, Linux and Windows packages.
//!
//! A zip is written in-process instead of by an external `zip`: the tool is
//! missing from many minimal Linux images and has no equivalent on a bare
//! Windows, and doing it here means one behavior everywhere that can be tested
//! on any host. Files are deflated as a stream (so a large asset is never held
//! in memory), carry their Unix permission bits (so an executable stays
//! executable after unzipping) and symbolic links are stored as links.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;

/// "Made by Unix, zip 2.0": the host byte that makes readers take the Unix
/// mode in the high half of the external attributes.
const MADE_BY_UNIX: u16 = (3 << 8) | 20;
/// File names are UTF-8, and the sizes follow the data (a streamed entry).
const FLAGS: u16 = 0x0800 | 0x0008;
const FLAGS_STORED: u16 = 0x0800;

const MODE_DIR: u32 = 0o040_755;
const MODE_FILE: u32 = 0o100_644;
const MODE_SYMLINK: u32 = 0o120_777;

/// What an entry of the archive holds.
enum Kind {
    Dir,
    File(PathBuf),
    Symlink(String),
}

struct Entry {
    /// The name inside the archive, with `/` separators.
    name: String,
    kind: Kind,
    mode: u32,
    modified: SystemTime,
}

/// What the central directory needs to know about a written entry.
struct Written {
    name: String,
    method: u16,
    flags: u16,
    time: u16,
    date: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    mode: u32,
    is_dir: bool,
    offset: u32,
}

/// Zip the folder `src` into `output`, under the single top-level folder
/// `top`. Entries are written in name order, so the same folder always gives
/// the same archive layout.
///
/// # Errors
///
/// Fails when a file cannot be read, or when the archive would need zip64
/// (4 GiB, or 65,535 entries), which this writer does not produce.
pub(super) fn create_zip(src: &Path, top: &str, output: &Path) -> anyhow::Result<()> {
    let mut entries = vec![Entry {
        name: format!("{top}/"),
        kind: Kind::Dir,
        mode: MODE_DIR,
        modified: modified(src),
    }];
    collect(src, top, &mut entries)?;
    if entries.len() > usize::from(u16::MAX) {
        bail!("{} has too many files for a .zip", src.display());
    }

    let file =
        std::fs::File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut out = Counter::new(io::BufWriter::new(file));
    let mut written = Vec::with_capacity(entries.len());
    for entry in &entries {
        written.push(write_entry(&mut out, entry)?);
    }

    let directory_start = u32::try_from(out.count).context("the .zip is too large")?;
    for item in &written {
        write_central(&mut out, item)?;
    }
    let directory_end = out.count;
    let directory_size = u32::try_from(directory_end - u64::from(directory_start))
        .context("the .zip directory is too large")?;
    let count = u16::try_from(written.len()).context("too many files for a .zip")?;
    put32(&mut out, END_OF_DIRECTORY)?;
    put16(&mut out, 0)?;
    put16(&mut out, 0)?;
    put16(&mut out, count)?;
    put16(&mut out, count)?;
    put32(&mut out, directory_size)?;
    put32(&mut out, directory_start)?;
    put16(&mut out, 0)?;
    out.inner.flush().context("writing the .zip")
}

/// Add everything under `dir` to `entries`, depth first and in name order.
fn collect(dir: &Path, prefix: &str, entries: &mut Vec<Entry>) -> anyhow::Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    children.sort_by_key(|child| child.file_name());

    for child in children {
        let path = child.path();
        let name = format!("{prefix}/{}", child.file_name().to_string_lossy());
        // `symlink_metadata`, so a link is archived as a link, not followed.
        let meta = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let modified = meta.modified().unwrap_or(UNIX_EPOCH);
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path)
                .with_context(|| format!("reading the link {}", path.display()))?;
            entries.push(Entry {
                name,
                kind: Kind::Symlink(target.to_string_lossy().into_owned()),
                mode: MODE_SYMLINK,
                modified,
            });
        } else if meta.is_dir() {
            entries.push(Entry {
                name: format!("{name}/"),
                kind: Kind::Dir,
                mode: unix_mode(&meta, MODE_DIR),
                modified,
            });
            collect(&path, &name, entries)?;
        } else {
            entries.push(Entry {
                name,
                kind: Kind::File(path),
                mode: unix_mode(&meta, MODE_FILE),
                modified,
            });
        }
    }
    Ok(())
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(UNIX_EPOCH)
}

#[cfg(unix)]
fn unix_mode(meta: &std::fs::Metadata, fallback: u32) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    // Keep the file-type bits of the fallback and the permissions of the file.
    (fallback & !0o7777) | (meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn unix_mode(_meta: &std::fs::Metadata, fallback: u32) -> u32 {
    fallback
}

fn write_entry<W: Write>(out: &mut Counter<W>, entry: &Entry) -> anyhow::Result<Written> {
    let offset = u32::try_from(out.count).context("the .zip is too large (needs zip64)")?;
    let (time, date) = dos_datetime(entry.modified);
    let is_dir = matches!(entry.kind, Kind::Dir);
    let streamed = matches!(entry.kind, Kind::File(_));
    let (method, flags) = if streamed {
        (8, FLAGS)
    } else {
        (0, FLAGS_STORED)
    };
    let mut item = Written {
        name: entry.name.clone(),
        method,
        flags,
        time,
        date,
        crc: 0,
        compressed: 0,
        size: 0,
        mode: entry.mode,
        is_dir,
        offset,
    };

    let name = entry.name.as_bytes();
    let name_len = u16::try_from(name.len()).context("a file name is too long for a .zip")?;
    match &entry.kind {
        Kind::File(path) => {
            local_header(out, &item, name_len)?;
            out.write_all(name)?;
            let file =
                std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
            let (crc, compressed, size) =
                deflate(file, out).with_context(|| format!("compressing {}", path.display()))?;
            item.crc = crc;
            item.compressed = compressed;
            item.size = size;
            // The sizes follow the data, which is what lets a large file stream.
            put32(out, DATA_DESCRIPTOR)?;
            put32(out, crc)?;
            put32(out, compressed)?;
            put32(out, size)?;
        }
        Kind::Dir | Kind::Symlink(_) => {
            let data = match &entry.kind {
                Kind::Symlink(target) => target.as_bytes(),
                _ => &[],
            };
            let mut crc = Crc::new();
            crc.update(data);
            item.crc = crc.sum();
            item.size = u32::try_from(data.len()).context("a link is too long for a .zip")?;
            item.compressed = item.size;
            local_header(out, &item, name_len)?;
            out.write_all(name)?;
            out.write_all(data)?;
        }
    }
    Ok(item)
}

fn local_header<W: Write>(out: &mut Counter<W>, item: &Written, name_len: u16) -> io::Result<()> {
    // A streamed entry does not know its CRC or sizes yet: they are zero here
    // and follow the data.
    let streamed = item.flags & 0x0008 != 0;
    put32(out, LOCAL_HEADER)?;
    put16(out, 20)?;
    put16(out, item.flags)?;
    put16(out, item.method)?;
    put16(out, item.time)?;
    put16(out, item.date)?;
    put32(out, if streamed { 0 } else { item.crc })?;
    put32(out, if streamed { 0 } else { item.compressed })?;
    put32(out, if streamed { 0 } else { item.size })?;
    put16(out, name_len)?;
    put16(out, 0)
}

fn write_central<W: Write>(out: &mut Counter<W>, item: &Written) -> anyhow::Result<()> {
    let name = item.name.as_bytes();
    put32(out, CENTRAL_HEADER)?;
    put16(out, MADE_BY_UNIX)?;
    put16(out, 20)?;
    put16(out, item.flags)?;
    put16(out, item.method)?;
    put16(out, item.time)?;
    put16(out, item.date)?;
    put32(out, item.crc)?;
    put32(out, item.compressed)?;
    put32(out, item.size)?;
    put16(out, u16::try_from(name.len())?)?;
    put16(out, 0)?; // extra field
    put16(out, 0)?; // comment
    put16(out, 0)?; // disk
    put16(out, 0)?; // internal attributes
    // The Unix mode in the high half, and the MS-DOS "directory" bit below it.
    put32(out, (item.mode << 16) | if item.is_dir { 0x10 } else { 0 })?;
    put32(out, item.offset)?;
    out.write_all(name)?;
    Ok(())
}

/// Deflate `source` into `out`, returning the CRC-32, the compressed size and
/// the original size.
fn deflate<R: Read, W: Write>(
    mut source: R,
    out: &mut Counter<W>,
) -> anyhow::Result<(u32, u32, u32)> {
    let before = out.count;
    let mut crc = Crc::new();
    let mut encoder = DeflateEncoder::new(&mut *out, Compression::default());
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crc.update(&buffer[..read]);
        encoder.write_all(&buffer[..read])?;
    }
    encoder.finish()?;
    let compressed =
        u32::try_from(out.count - before).context("a file is too large (needs zip64)")?;
    Ok((crc.sum(), compressed, crc.amount()))
}

/// MS-DOS time and date, which is what a zip stores. The format covers 1980 to
/// 2107, and has two-second resolution.
pub(super) fn dos_datetime(time: SystemTime) -> (u16, u16) {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days);
    if year < 1980 {
        return (0, (1 << 5) | 1); // 1980-01-01 00:00:00
    }
    let year = year.min(2107);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    let time = ((rest / 3600) << 11) | (((rest % 3600) / 60) << 5) | ((rest % 60) / 2);
    (time as u16, date as u16)
}

/// Year, month and day of a day count since 1970-01-01 (the proleptic
/// Gregorian calendar; Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// A writer that counts what goes through it, for the offsets of a zip.
struct Counter<W: Write> {
    inner: W,
    count: u64,
}

impl<W: Write> Counter<W> {
    fn new(inner: W) -> Self {
        Self { inner, count: 0 }
    }
}

impl<W: Write> Write for Counter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn put16<W: Write>(out: &mut W, value: u16) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn put32<W: Write>(out: &mut W, value: u32) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

/// `tar` writing `output` from the folder `top` inside `parent`.
///
/// `COPYFILE_DISABLE` keeps macOS's `tar` from adding `._*` files for extended
/// attributes, which would show up as junk on any other system.
pub(super) fn tar_command(parent: &Path, top: &str, output: &Path) -> Command {
    let mut cmd = Command::new("tar");
    cmd.arg("-czf")
        .arg(output)
        .arg("-C")
        .arg(parent)
        .arg(top)
        .env("COPYFILE_DISABLE", "1");
    cmd
}

/// Copy `src` into `dest`, creating it, directories and all.
pub(super) fn copy_dir(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            // `copy` carries the permission bits over, so the executable stays
            // executable.
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
