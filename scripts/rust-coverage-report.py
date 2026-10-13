import argparse
import json
import re
import subprocess
from pathlib import Path


def generated_command_regions(report):
    candidates = {}
    source_lines = {}
    for data in report["data"]:
        for function in data.get("functions", []):
            if len(function["regions"]) != 1 or function.get("count", 0) != 0:
                continue
            for start, column, end, end_column, count, file_id, _, kind in function["regions"]:
                if start != end or end_column != column + 1 or count != 0 or kind != 0:
                    continue
                path = function["filenames"][file_id]
                if path not in source_lines:
                    source_lines[path] = Path(path).read_text().splitlines()
                lines = source_lines[path]
                if start > len(lines):
                    continue
                attribute = lines[start - 1].rstrip()
                if column == len(attribute) and re.fullmatch(
                    r"\s*#\[tauri::command(?:\([^\n]*\))?\]", attribute
                ):
                    candidates.setdefault((path, start), set()).add(function["name"])
    return candidates


def normalize_lcov(lcov, ignored):
    records = []
    for record in lcov.split("end_of_record\n"):
        source = None
        lines = []
        removed = False
        names = set()
        for line in record.splitlines():
            if line.startswith("SF:"):
                source = line[3:]
                names = {name for (path, _), functions in ignored.items() if path == source for name in functions}
        for line in record.splitlines(keepends=True):
            if line.startswith("SF:"):
                source = line[3:].rstrip("\n")
            if line.startswith("DA:"):
                number, count, *_ = line[3:].strip().split(",")
                if count == "0" and (source, int(number)) in ignored:
                    removed = True
                    continue
            if line.startswith("FN:") and line.rstrip("\n").split(",", 1)[1] in names:
                removed = True
                continue
            if line.startswith("FNDA:") and line.rstrip("\n").split(",", 1)[1] in names:
                removed = True
                continue
            lines.append(line)
        if removed:
            counts = [int(line[3:].split(",")[1]) for line in lines if line.startswith("DA:")]
            normalized = []
            for line in lines:
                if line.startswith("LF:"):
                    line = f"LF:{len(counts)}\n"
                elif line.startswith("LH:"):
                    line = f"LH:{sum(count > 0 for count in counts)}\n"
                elif line.startswith("FNF:"):
                    line = f"FNF:{sum(item.startswith('FN:') for item in lines)}\n"
                elif line.startswith("FNH:"):
                    line = f"FNH:{sum(int(item[5:].split(',')[0]) > 0 for item in lines if item.startswith('FNDA:'))}\n"
                normalized.append(line)
            lines = normalized
        records.append("".join(lines))
    return "end_of_record\n".join(records)


def current_diff(base):
    ancestor = subprocess.check_output(["git", "merge-base", base, "HEAD"]).strip().decode()
    patch = subprocess.check_output(["git", "diff", ancestor, "--", "src-tauri/src"])
    for name in subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z", "--", "src-tauri/src"]).split(b"\0"):
        if name:
            diff = subprocess.run(["git", "diff", "--no-index", "--", "/dev/null", name.decode()], stdout=subprocess.PIPE, check=False)
            if diff.returncode not in (0, 1):
                raise subprocess.CalledProcessError(diff.returncode, diff.args)
            patch += diff.stdout
    return patch


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("report_dir", type=Path)
    parser.add_argument("base")
    args = parser.parse_args()
    report = json.loads((args.report_dir / "raw.json").read_text())
    lcov = (args.report_dir / "lcov.info").read_text()
    (args.report_dir / "diff-lcov.info").write_text(normalize_lcov(lcov, generated_command_regions(report)))
    (args.report_dir / "changes.diff").write_bytes(current_diff(args.base))


if __name__ == "__main__":
    main()
