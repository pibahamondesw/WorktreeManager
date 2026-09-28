use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use super::runtime;

static PREPARATION: Mutex<()> = Mutex::new(());
const WORKBENCH: &str = "lib/vscode/out/vs/code/browser/workbench/workbench.js";
const PTY_HOST: &str = "lib/vscode/out/vs/platform/terminal/node/ptyHostMain.js";
const PRELOADS: &[(&str, &str)] = &[
    ("lib/vscode/out/server-main.js", "../../../session.cjs"),
    (PTY_HOST, "../../../../../../../session.cjs"),
];
const PATCHES: &[(&str, &str, &str)] = &[
    (WORKBENCH, "options:r};return nW.set(e,c,l)", "options:{...r,extensionHostEnv:{...r?.extensionHostEnv,...globalThis.__wtmSessionEnv}}};return nW.set(e,c,l)"),
    (WORKBENCH, "e=t,eU.complete(t)", "e=t,globalThis.__wtmShutdown=()=>t.shutdown(),eU.complete(t)"),
    (PTY_HOST, "import{spawn as Pd}from\"node-pty\";", "import{spawn as wtmSpawn}from\"node-pty\";const Pd=(...args)=>globalThis.__wtmSpawnPty(wtmSpawn,...args);"),
];

pub fn prepare(root: &Path) -> Result<PathBuf, String> {
    let _guard = PREPARATION.lock().map_err(|error| error.to_string())?;
    let source = runtime::binary(root)?
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let fingerprint = format!(
        "{:x}",
        md5::compute(format!(
            "{PATCHES:?}{PRELOADS:?}{}",
            include_str!("session.cjs")
        ))
    );
    let target = source.with_file_name(format!(
        "wtm-{}-{fingerprint}",
        source.file_name().unwrap().to_string_lossy()
    ));
    if target.join("wtm-ready").is_file() {
        return Ok(target.join("bin/code-server"));
    }
    let temporary = target.with_extension(runtime::random_id()?);
    let result = (|| {
        let output = Command::new("/bin/cp")
            .arg("-cR")
            .arg(&source)
            .arg(&temporary)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "Could not prepare shared editor runtime: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        for (file, before, after) in PATCHES {
            let path = temporary.join(file);
            let source = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            fs::write(path, replace_once(&source, before, after)?)
                .map_err(|error| error.to_string())?;
        }
        for (file, preload) in PRELOADS {
            let path = temporary.join(file);
            let source = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            fs::write(path, format!("import {preload:?};\n{source}"))
                .map_err(|error| error.to_string())?;
        }
        fs::write(temporary.join("session.cjs"), include_str!("session.cjs"))
            .map_err(|error| error.to_string())?;
        fs::write(temporary.join("wtm-ready"), runtime::VERSION)
            .map_err(|error| error.to_string())?;
        fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
        Ok(target.join("bin/code-server"))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(temporary);
    }
    result
}

fn replace_once(source: &str, before: &str, after: &str) -> Result<String, String> {
    if source.matches(before).count() != 1 {
        return Err(format!("The shared editor integration does not match code-server {}. Repair or update the editor before opening tasks.", runtime::VERSION));
    }
    Ok(source.replacen(before, after, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_or_ambiguous_runtime_is_not_patched() {
        assert_eq!(replace_once("abc", "b", "B").unwrap(), "aBc");
        assert!(replace_once("abc", "d", "D").is_err());
        assert!(replace_once("bb", "b", "B").is_err());
    }

    #[test]
    fn prepares_a_reusable_copy_without_changing_the_original_runtime() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        let binary = runtime::binary(&root).unwrap();
        let original = binary.parent().unwrap().parent().unwrap();
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "runtime").unwrap();
        for file in [WORKBENCH, PTY_HOST, "lib/vscode/out/server-main.js"] {
            let path = original.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let source = PATCHES
                .iter()
                .filter(|(target, _, _)| *target == file)
                .map(|(_, before, _)| *before)
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(path, source).unwrap();
        }
        let prepared = prepare(&root).unwrap();
        assert_ne!(prepared, binary);
        assert_eq!(prepare(&root).unwrap(), prepared);
        let directory = prepared.parent().unwrap().parent().unwrap();
        for (file, before, after) in PATCHES {
            assert!(fs::read_to_string(original.join(file))
                .unwrap()
                .contains(before));
            assert!(fs::read_to_string(directory.join(file))
                .unwrap()
                .contains(after));
        }
        for (file, preload) in PRELOADS {
            assert!(fs::read_to_string(directory.join(file))
                .unwrap()
                .starts_with(&format!("import {preload:?};")));
        }
        assert_eq!(
            fs::read_to_string(directory.join("session.cjs")).unwrap(),
            include_str!("session.cjs")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
