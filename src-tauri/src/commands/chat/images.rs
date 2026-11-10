//! Images dropped on the composer arrive as paths from the native drop handler; their bytes are
//! read here so they attach exactly like pasted ones.

use std::fs;
use std::path::Path;

use base64::Engine;
use serde::Serialize;

const MAX_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedImage {
    pub name: String,
    pub media_type: String,
    pub data: String,
}

fn media_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

pub fn read(path: &Path) -> Result<DroppedImage, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let media_type = media_type(path).ok_or_else(|| format!("Not a supported image: {name}"))?;
    let size = fs::metadata(path)
        .map_err(|e| format!("Could not read {name}: {e}"))?
        .len();
    if size > MAX_BYTES {
        return Err(format!("Images must be 10 MB or smaller: {name}"));
    }
    let bytes = fs::read(path).map_err(|e| format!("Could not read {name}: {e}"))?;
    Ok(DroppedImage {
        name,
        media_type: media_type.to_string(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_supported_images_as_base64() {
        let dir = std::env::temp_dir().join(format!("wtm-dropped-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("Photo.JPG");
        fs::write(&photo, "hi").unwrap();
        let notes = dir.join("notes.txt");
        fs::write(&notes, "hi").unwrap();

        assert_eq!(
            read(&photo).unwrap(),
            DroppedImage {
                name: "Photo.JPG".into(),
                media_type: "image/jpeg".into(),
                data: "aGk=".into(),
            }
        );
        assert_eq!(
            read(&notes).unwrap_err(),
            "Not a supported image: notes.txt"
        );
        assert!(read(&dir.join("gone.png"))
            .unwrap_err()
            .starts_with("Could not read gone.png"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_images_over_the_size_limit() {
        let dir = std::env::temp_dir().join(format!("wtm-dropped-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let huge = dir.join("huge.png");
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();

        assert_eq!(
            read(&huge).unwrap_err(),
            "Images must be 10 MB or smaller: huge.png"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
