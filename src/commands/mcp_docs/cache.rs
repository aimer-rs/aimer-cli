use super::*;

pub(super) fn decode_json_payload(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        if bytes.len() as u64 > MAX_COMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_COMPRESSED_BYTES}-byte gzip size limit");
        }
        let decoder = GzDecoder::new(bytes);
        let mut output = Vec::new();
        decoder
            .take(MAX_UNCOMPRESSED_BYTES + 1)
            .read_to_end(&mut output)
            .context("decompressing docs.rs Rustdoc JSON")?;
        if output.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte decompressed size limit");
        }
        Ok(output)
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        if bytes.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte zstd size limit");
        }
        let decoder = zstd::stream::read::Decoder::new(bytes)
            .context("decompressing cached zstd Rustdoc JSON")?;
        let mut output = Vec::new();
        decoder
            .take(MAX_UNCOMPRESSED_BYTES + 1)
            .read_to_end(&mut output)
            .context("decompressing cached zstd Rustdoc JSON")?;
        if output.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte decompressed size limit");
        }
        Ok(output)
    } else if bytes
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .next()
        == Some(b'{')
    {
        if bytes.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte decompressed size limit");
        }
        Ok(bytes.to_vec())
    } else {
        bail!("docs.rs response is neither gzip data nor Rustdoc JSON");
    }
}

pub(super) fn normalize_compressed_json(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    let compressed = encoder.finish()?;
    if compressed.len() as u64 > MAX_COMPRESSED_BYTES {
        bail!("documentation exceeds the {MAX_COMPRESSED_BYTES}-byte cache entry limit");
    }
    Ok(compressed)
}

pub(super) fn compress_zstd_json(json: &[u8]) -> anyhow::Result<Vec<u8>> {
    if json.len() as u64 > MAX_UNCOMPRESSED_BYTES {
        bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte size limit");
    }
    zstd::bulk::compress(json, 3).context("compressing Rustdoc JSON with zstd")
}

pub(super) fn read_cache_entry(
    path: &Path,
    version: &str,
    max_entry_bytes: u64,
) -> Result<Option<(RustdocCrate, Vec<u8>)>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read cached docs {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.len() > max_entry_bytes {
        let _ = fs::remove_file(path);
        return Err(format!(
            "cached docs {} exceeded the {max_entry_bytes}-byte limit and were removed",
            path.display()
        ));
    }

    let bytes = fs::read(path)
        .map_err(|error| format!("could not read cached docs {}: {error}", path.display()))?;
    match parse_compressed_index(&bytes, version) {
        Ok(rustdoc) => Ok(Some((rustdoc, bytes))),
        Err(error) => {
            let _ = fs::remove_file(path);
            Err(format!(
                "cached docs {} were invalid and removed: {error:#}",
                path.display()
            ))
        }
    }
}

pub(super) struct CacheFile {
    pub(super) path: PathBuf,
    pub(super) size: u64,
    pub(super) modified: SystemTime,
}

pub(super) fn collect_cache_files(root: &Path, files: &mut Vec<CacheFile>) -> anyhow::Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", root.display())),
    };
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_cache_files(&entry.path(), files)?;
        } else if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".json.gz") || name.ends_with(".json.zst"))
        {
            let metadata = entry.metadata()?;
            files.push(CacheFile {
                path: entry.path(),
                size: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    Ok(())
}

pub(super) fn touch_cache_file(path: &Path) -> std::io::Result<()> {
    let file = File::open(path)?;
    file.set_times(FileTimes::new().set_modified(SystemTime::now()))
}
