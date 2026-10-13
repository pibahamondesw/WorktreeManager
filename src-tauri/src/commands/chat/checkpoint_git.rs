use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commands::git::git_command;

#[derive(Clone, Serialize, Deserialize)]
pub struct BaseFile {
    pub object: String,
    pub hash: String,
    pub mode: u32,
}

pub fn read_base(folder: &str, head: &str) -> Result<BTreeMap<String, BaseFile>, String> {
    let output = git_command()
        .args(["-C", folder, "ls-tree", "-rz", "--full-tree", head])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    let mut entries = BTreeMap::new();
    for entry in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let separator = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or("Invalid Git tree entry")?;
        let (metadata, path) = entry.split_at(separator);
        let path = &path[1..];
        let metadata = std::str::from_utf8(metadata).map_err(|e| e.to_string())?;
        let mut parts = metadata.split_whitespace();
        let mode = u32::from_str_radix(parts.next().ok_or("Git tree entry has no mode")?, 8)
            .map_err(|e| e.to_string())?;
        let kind = parts.next();
        let object = parts.next().ok_or("Git tree entry has no object")?;
        if kind != Some("blob") {
            continue;
        }
        let path =
            std::str::from_utf8(path).map_err(|_| "Non-UTF-8 file names cannot be checkpointed")?;
        entries.insert(path.to_string(), (object.to_string(), mode));
    }
    let objects = entries
        .values()
        .map(|(object, _)| object.clone())
        .collect::<BTreeSet<_>>();
    let hashes = hash_objects(folder, objects)?;
    entries
        .into_iter()
        .map(|(path, (object, mode))| {
            let hash = hashes
                .get(&object)
                .ok_or("Git object hash is missing")?
                .clone();
            Ok((path, BaseFile { object, hash, mode }))
        })
        .collect()
}

fn hash_objects(
    folder: &str,
    objects: BTreeSet<String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut child = git_command()
        .args(["-C", folder, "cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("Git stdin is unavailable")?;
    let requests = objects.clone();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for object in requests {
            writeln!(stdin, "{object}")?;
        }
        Ok(())
    });
    let result = (|| -> Result<_, String> {
        let mut stdout = BufReader::new(child.stdout.take().ok_or("Git stdout is unavailable")?);
        let mut hashes = BTreeMap::new();
        for object in objects {
            let mut header = String::new();
            stdout.read_line(&mut header).map_err(|e| e.to_string())?;
            let mut parts = header.split_whitespace();
            if parts.next() != Some(object.as_str()) || parts.next() != Some("blob") {
                return Err("Git checkpoint object is unavailable".into());
            }
            let size = parts
                .next()
                .ok_or("Git object has no size")?
                .parse::<u64>()
                .map_err(|e| e.to_string())?;
            let mut hasher = Sha256::new();
            let mut remaining = size;
            let mut buffer = [0; 65536];
            while remaining > 0 {
                let count = remaining.min(buffer.len() as u64) as usize;
                stdout
                    .read_exact(&mut buffer[..count])
                    .map_err(|e| e.to_string())?;
                hasher.update(&buffer[..count]);
                remaining -= count as u64;
            }
            let mut newline = [0];
            stdout.read_exact(&mut newline).map_err(|e| e.to_string())?;
            if newline != *b"\n" {
                return Err("Invalid Git object separator".into());
            }
            hashes.insert(object, format!("{:x}", hasher.finalize()));
        }
        Ok(hashes)
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string());
    let written = writer
        .join()
        .map_err(|_| "Git checkpoint writer failed")?
        .map_err(|e| e.to_string());
    let hashes = result?;
    written?;
    if !status?.success() {
        return Err("Could not read the Git checkpoint base".into());
    }
    Ok(hashes)
}

pub fn read_blob(folder: &str, object: &str) -> Result<Vec<u8>, String> {
    let output = git_command()
        .args(["-C", folder, "cat-file", "blob", object])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Git checkpoint object is unavailable; no files were restored".into());
    }
    Ok(output.stdout)
}
