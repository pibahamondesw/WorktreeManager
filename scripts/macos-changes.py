import argparse
import fnmatch
import os
import posixpath
import re
import subprocess
import sys
from pathlib import Path

LIBRARY_ROOT = "src-tauri/src"
MODULE_ROOT_FILES = {"lib.rs", "main.rs", "mod.rs"}
MACOS_CONTENT = re.compile(
    r'target_os\s*=\s*"macos"'
    r"|\bobjc2\b|\bmsg_send(?:_id)?!|\bAnyObject\b|\bAnyClass\b|\bNS[A-Z][A-Za-z]+\b"
)
MACOS_CFG = re.compile(r'^#\[cfg\((?!.*\bnot\()[^\]]*target_os\s*=\s*"macos"')
PATH_ATTRIBUTE = re.compile(r'^#\[path\s*=\s*"([^"]+)"\]')
MODULE_DECLARATION = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
NATIVE_AREAS = (
    "src-tauri/src/commands/login_shell.rs",
    "src-tauri/src/commands/agent_notifications.rs",
    "src-tauri/src/commands/embedded/",
    "src-tauri/src/menu.rs",
    "src-tauri/src/menu/",
    "src-tauri/src/lifecycle.rs",
    "src-tauri/tests/menu.rs",
)
BUILD_INPUTS = (
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri*.conf.json",
    ".github/workflows/ci.yml",
    "scripts/macos-changes.py",
    "scripts/rust-coverage.sh",
    "scripts/rust-coverage-report.py",
)


def git(*args):
    return subprocess.run(["git", *args], stdout=subprocess.PIPE, check=True).stdout


def changed_paths(ancestor):
    tracked = git("diff", "--no-renames", "--name-only", "-z", ancestor)
    untracked = git("ls-files", "--others", "--exclude-standard", "-z")
    return sorted({name for name in (tracked + untracked).decode().split("\0") if name})


def base_content(ancestor, path):
    result = subprocess.run(
        ["git", "show", f"{ancestor}:{path}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.stdout.decode(errors="replace") if result.returncode == 0 else ""


def current_content(path):
    file = Path(path)
    return file.read_text(errors="replace") if file.is_file() else ""


def module_file(declaring_file, name, path_attribute):
    directory = posixpath.dirname(declaring_file)
    if path_attribute:
        return [posixpath.normpath(posixpath.join(directory, path_attribute))]
    if posixpath.basename(declaring_file) not in MODULE_ROOT_FILES:
        directory = posixpath.join(directory, posixpath.splitext(posixpath.basename(declaring_file))[0])
    return [posixpath.join(directory, f"{name}.rs"), posixpath.join(directory, name, "mod.rs")]


def module_subtree(file):
    if posixpath.basename(file) == "mod.rs":
        return posixpath.dirname(file) + "/"
    return posixpath.splitext(file)[0] + "/"


def macos_modules(declaring_file, source):
    modules = []
    attributes = []
    for line in source.splitlines():
        line = line.strip()
        if line.startswith("#["):
            attributes.append(line)
            continue
        if not line or line.startswith("//"):
            continue
        declaration = MODULE_DECLARATION.match(line)
        if declaration and any(MACOS_CFG.match(attribute) for attribute in attributes):
            path = next((m.group(1) for a in attributes if (m := PATH_ATTRIBUTE.match(a))), None)
            modules.extend(module_file(declaring_file, declaration.group(1), path))
        attributes = []
    return modules


def macos_module_files():
    files = set()
    for path in sorted(Path(LIBRARY_ROOT).rglob("*.rs")):
        files.update(macos_modules(path.as_posix(), path.read_text(errors="replace")))
    return files


def in_native_area(path):
    return any(path == area or (area.endswith("/") and path.startswith(area)) for area in NATIVE_AREAS)


def within_macos_module(path, modules):
    return any(path == module or path.startswith(module_subtree(module)) for module in modules)


def reason(ancestor, path, modules):
    if any(fnmatch.fnmatchcase(path, pattern) for pattern in BUILD_INPUTS):
        return "build input"
    if not path.endswith(".rs"):
        return None
    if in_native_area(path):
        return "native area"
    if within_macos_module(path, modules):
        return "macOS-only module"
    if any(MACOS_CONTENT.search(source) for source in (current_content(path), base_content(ancestor, path))):
        return "macOS code"
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("base")
    args = parser.parse_args()
    os.chdir(git("rev-parse", "--show-toplevel").decode().strip())
    ancestor = git("merge-base", args.base, "HEAD").decode().strip()
    modules = macos_module_files()
    for path in changed_paths(ancestor):
        cause = reason(ancestor, path, modules)
        if cause:
            print(path, flush=True)
            print(f"{path}: {cause}", file=sys.stderr)


if __name__ == "__main__":
    main()
