use std::path::Path;

use base64::Engine;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn decode_png(base64_png: &str) -> Result<Vec<u8>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_png)
        .map_err(|error| format!("Invalid image data: {error}"))?;
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err("Image data is not a PNG".into());
    }
    Ok(bytes)
}

fn is_png_destination(path: &Path) -> bool {
    path.is_absolute()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

/// Writes a PNG to the destination the user picked in the save dialog. Only `.png`
/// files with PNG content can be written.
#[tauri::command]
pub async fn save_png(path: String, base64_png: String) -> Result<(), String> {
    let path = Path::new(&path);
    if !is_png_destination(path) {
        return Err("The destination must be an absolute .png path".into());
    }
    let bytes = decode_png(&base64_png)?;
    std::fs::write(path, bytes)
        .map_err(|error| format!("Could not save {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wtm-export-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn encode(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn png() -> Vec<u8> {
        [PNG_SIGNATURE, b"rest"].concat()
    }

    fn save(path: &Path, base64_png: String) -> Result<(), String> {
        tauri::async_runtime::block_on(save_png(path.display().to_string(), base64_png))
    }

    #[test]
    fn writes_png_data_to_the_chosen_destination() {
        let dir = temp_dir("writes");
        let path = dir.join("diagram.PNG");

        save(&path, encode(&png())).unwrap();

        assert_eq!(std::fs::read(path).unwrap(), png());
    }

    #[test]
    fn rejects_destinations_that_are_not_absolute_png_paths() {
        let dir = temp_dir("rejects-destinations");

        for path in [dir.join("diagram.svg"), "diagram.png".into()] {
            assert_eq!(
                save(&path, encode(&png())).unwrap_err(),
                "The destination must be an absolute .png path"
            );
            assert!(!path.exists());
        }
    }

    #[test]
    fn rejects_data_that_is_not_a_png() {
        let dir = temp_dir("rejects-data");
        let path = dir.join("diagram.png");

        assert_eq!(
            save(&path, encode(b"<svg></svg>")).unwrap_err(),
            "Image data is not a PNG"
        );
        assert!(save(&path, "not base64!".into())
            .unwrap_err()
            .starts_with("Invalid image data"));
        assert!(!path.exists());
    }

    #[test]
    fn reports_write_failures() {
        let dir = temp_dir("write-failures");
        let path = dir.join("missing").join("diagram.png");

        assert!(save(&path, encode(&png()))
            .unwrap_err()
            .starts_with("Could not save"));
    }
}
