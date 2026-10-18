#!/usr/bin/env python3
"""生成 Windows x64 第三方声明；仅开发期使用 Python 标准库和离线 Cargo。

示例（从任意目录运行，--cargo 必须是绝对路径）：
  python tools/generate-third-party-notices.py --cargo <absolute-cargo.exe>
  python tools/generate-third-party-notices.py --cargo <absolute-cargo.exe> --check

不构建、不下载、不修改锁文件。依赖更新出现未知许可或缺失文本时失败，
应审查实际注册表源码后再扩展此脚本，不使用网络模板或猜测的许可文本。
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit

TARGET = "x86_64-pc-windows-msvc"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
LICENSE_MARKERS = {
    "MIT": ("permission is hereby granted, free of charge",),
    "Apache-2.0": ("apache license", "version 2.0, january 2004"),
    "ISC": ("permission to use, copy, modify, and/or distribute",),
    "BSD-3-Clause": ("redistribution and use in source and binary forms", "neither the name"),
    "Zlib": ("the origin of this software must not be misrepresented",),
    "Unicode-3.0": ("unicode license v3",),
    "Unlicense": ("this is free and unencumbered software released into the public domain",),
}
LEGAL_NAME = re.compile(
    r"^(?:licen[cs]e|copying|copyright|notice|unlicense)(?:$|[._-])", re.I
)
LEGAL_CONTENT = re.compile(
    r"copyright|public.domain|permission (?:is hereby|to use)|"
    r"redistribution and use|licensed under|spdx-license", re.I
)
REQUIRED_BUNDLED = {
    "ring": {
        "LICENSE", "LICENSE-BoringSSL", "LICENSE-other-bits",
        "src/polyfill/once_cell/LICENSE-APACHE",
        "src/polyfill/once_cell/LICENSE-MIT", "third_party/fiat/LICENSE",
    },
    "atomic-waker": {"LICENSE-THIRD-PARTY"},
    "tracing-core": {"src/spin/LICENSE"},
}


class NoticeError(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise NoticeError(message)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def normalize_text(data: bytes) -> str:
    return data.decode("utf-8-sig").replace("\r\n", "\n").replace("\r", "\n")


def normalized_sha256(data: bytes) -> str:
    return sha256(normalize_text(data).encode("utf-8"))


def read_text(path: Path) -> tuple[bytes, str]:
    data = path.read_bytes()
    text = normalize_text(data)
    require(bool(text.strip()), f"Empty evidence file: {path.name}")
    require("\x00" not in text, f"Non-text evidence file: {path.name}")
    return data, text


def license_ids(expression: str) -> set[str]:
    # Cargo's historical MIT/Apache-2.0 spelling means an OR choice.
    normalized = expression.replace("/", " OR ")
    tokens = re.findall(r"[A-Za-z0-9.-]+|[()]", normalized)
    require("".join(tokens) == re.sub(r"\s", "", normalized),
            f"Unsupported license expression: {expression}")
    ids = set(tokens) - {"AND", "OR", "(", ")"}
    require(bool(ids) and ids <= LICENSE_MARKERS.keys(),
            f"Unknown license expression: {expression}")
    # Validate the expression grammar too; WITH exceptions need explicit review.
    position = 0

    def term() -> None:
        nonlocal position
        require(position < len(tokens), f"Incomplete license: {expression}")
        token = tokens[position]
        position += 1
        if token == "(":
            group()
            require(position < len(tokens) and tokens[position] == ")",
                    f"Unbalanced license: {expression}")
            position += 1
        else:
            require(token in LICENSE_MARKERS, f"Unsupported license: {expression}")

    def group() -> None:
        nonlocal position
        term()
        while position < len(tokens) and tokens[position] in {"AND", "OR"}:
            position += 1
            term()

    group()
    require(position == len(tokens), f"Invalid license expression: {expression}")
    return ids


def metadata(cargo: Path, project: Path) -> tuple[dict, bytes, bytes]:
    require(cargo.is_absolute() and cargo.is_file(), "--cargo must name an existing absolute executable")
    manifest = (project / "Cargo.toml").read_bytes()
    lock = (project / "Cargo.lock").read_bytes()
    result = subprocess.run(
        [str(cargo), "metadata", "--offline", "--locked", "--format-version", "1",
         "--filter-platform", TARGET, "--manifest-path", str(project / "Cargo.toml")],
        cwd=project, capture_output=True, encoding="utf-8", timeout=120,
    )
    require(result.returncode == 0,
            "Offline locked metadata failed; ensure the parent version/lock update is ready.\n"
            + result.stderr.strip())
    require(manifest == (project / "Cargo.toml").read_bytes()
            and lock == (project / "Cargo.lock").read_bytes(),
            "Manifest or lockfile changed during metadata; retry after the parent is ready")
    return json.loads(result.stdout), manifest, lock


def dependency_closure(meta: dict) -> tuple[dict, list[dict], dict[str, set[str]]]:
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root_id = meta["resolve"]["root"]
    require(root_id in packages, "Expected a single root Cargo package")
    root = packages[root_id]

    # Keep separate reachability states: proc-macro dependencies are build-time too.
    roles: dict[str, set[str]] = {}
    todo = [(root_id, "normal")]
    while todo:
        package_id, role = todo.pop()
        if role in roles.setdefault(package_id, set()):
            continue
        roles[package_id].add(role)
        for dep in nodes[package_id]["deps"]:
            for kind in dep["dep_kinds"]:
                if kind["kind"] not in (None, "build"):
                    continue
                proc_macro = any("proc-macro" in t["kind"] for t in packages[dep["pkg"]]["targets"])
                child_role = "build" if role == "build" or kind["kind"] == "build" or proc_macro else "normal"
                todo.append((dep["pkg"], child_role))
    selected = []
    for package_id in roles:
        if package_id == root_id:
            continue
        p = packages[package_id]
        require(p["source"] == REGISTRY,
                f"Unreviewed non-crates.io dependency: {p['name']} {p['version']}")
        selected.append(p)
    selected.sort(key=lambda p: (p["name"], p["version"]))
    return root, selected, roles


def safe_url(value: str | None) -> str:
    if not value:
        return "-"
    url = urlsplit(value)
    require(url.scheme in {"http", "https"} and bool(url.hostname)
            and not url.username and not url.password and not url.query and not url.fragment,
            "Non-public or unsupported dependency homepage/repository URL")
    require("|" not in value and "\n" not in value and "\r" not in value,
            "Unsafe dependency URL")
    return value


def evidence_file(root: Path, relative: str) -> tuple[str, bytes, str]:
    path = root / relative
    require(path.resolve().is_relative_to(root.resolve()), "Evidence path escapes crate directory")
    data, text = read_text(path)
    return relative, data, text


def license_files(package: dict) -> list[tuple[str, bytes, str]]:
    root = Path(package["manifest_path"]).parent
    name = package["name"]
    expression = package.get("license")
    require(bool(expression), f"Missing SPDX metadata: {name} {package['version']}")
    ids = license_ids(expression)
    files = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or not LEGAL_NAME.match(path.name):
            continue
        rel = path.relative_to(root).as_posix()
        if name == "libsqlite3-sys" and rel.startswith("sqlcipher/"):
            # This release enables bundled SQLite, not the mutually exclusive SQLCipher.
            continue
        files.append(evidence_file(root, rel))
    if package.get("license_file"):
        rel = Path(package["license_file"]).as_posix()
        if rel not in {f[0] for f in files}:
            files.append(evidence_file(root, rel))
    require(bool(files), f"No installed license text: {name} {package['version']}")
    paths = {f[0] for f in files}
    require(REQUIRED_BUNDLED.get(name, set()) <= paths,
            f"Missing bundled license evidence: {name}")
    combined = "\n".join(f[2].lower() for f in files)
    combined = re.sub(r"\s+", " ", combined)
    for identifier in ids:
        recognized = all(marker in combined for marker in LICENSE_MARKERS[identifier])
        if identifier == "Apache-2.0" and not recognized:
            # shlex ships the Apache grant/attribution, not the full terms. The
            # aggregate must also contain actual full Apache text from a selected
            # crate, checked below; never substitute a fabricated template.
            recognized = "licensed under the apache license, version 2.0" in combined
        require(recognized,
                f"No {identifier} license evidence recognized for {name} {package['version']}")
    return sorted(files)


def leading_comments(text: str, suffix: str) -> str:
    """Copy comments only, stopping before code (never copy keys/certificates)."""
    lines = text.splitlines(keepends=True)
    result = []
    in_block = False
    for line in lines:
        stripped = line.lstrip()
        if in_block:
            result.append(line)
            if "*/" in line:
                require(not line.split("*/", 1)[1].strip(), "Code shares a license comment line")
                in_block = False
        elif stripped.startswith("/*"):
            result.append(line)
            if "*/" in stripped:
                require(not stripped.split("*/", 1)[1].strip(), "Code shares a license comment line")
            else:
                in_block = True
        elif stripped.startswith("//"):
            result.append(line)
        elif suffix == ".pl" and stripped.startswith("#"):
            if not stripped.startswith("#!"):
                result.append(line)
        elif not stripped.strip():
            result.append(line)
        else:
            break
    require(not in_block, "Unterminated source license comment")
    return "".join(result).rstrip()


def bundled_evidence(package: dict, features: list[str]) -> list[tuple[str, bytes, str]]:
    root = Path(package["manifest_path"]).parent
    name = package["name"]
    evidence = []
    if name == "ring":
        # Includes other architectures conservatively. Only leading legal comments
        # from implementation/generator sources, never tests, PEM, DER, p8 or objects.
        for directory in ("crypto", "include", "third_party", "src"):
            for path in sorted((root / directory).rglob("*")):
                if path.suffix not in {".c", ".h", ".S", ".rs", ".pl", ".inl"}:
                    continue
                rel = path.relative_to(root)
                if any("test" in part.lower() or part == "bench" for part in rel.parts):
                    continue
                data, text = read_text(path)
                header = leading_comments(text, path.suffix)
                if LEGAL_CONTENT.search(header):
                    evidence.append((rel.as_posix() + " [leading legal comments]", data, header))
        require((root / "third_party/fiat/curve25519_64_msvc.h").is_file(),
                "Missing ring bundled Windows Fiat implementation")
        require("crypto/curve25519/curve25519.c" in {f[0].split(" [", 1)[0] for f in evidence},
                "Missing ring bundled source attribution headers")
    elif name == "libsqlite3-sys":
        require("bundled" in features and not any("sqlcipher" in f for f in features),
                "SQLite feature selection changed; review bundled licensing")
        for rel in ("sqlite3/sqlite3.c", "sqlite3/sqlite3.h"):
            data, text = read_text(root / rel)
            if rel.endswith(".c"):
                version = re.search(r"SQLite\n\*\* version ([0-9.]+)", text)
                require(version is not None, "Unrecognized bundled SQLite version header")
                evidence.append((rel + " [amalgamation origin]", data, text[:text.index("*/") + 2]))
            blocks = re.findall(r"/\*.*?\*/", text, re.S)
            legal_blocks = sorted({b for b in blocks if LEGAL_CONTENT.search(b)})
            require(any("The author disclaims copyright" in b for b in legal_blocks),
                    "Missing SQLite public-domain copyright disclaimer")
            for index, block in enumerate(legal_blocks, 1):
                evidence.append((f"{rel} [legal comment {index}]", data, block))
    return evidence


def generate(meta: dict, manifest: bytes, lock: bytes) -> tuple[str, int, Counter]:
    root, packages, roles = dependency_closure(meta)
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    counts = Counter(p["license"] for p in packages)
    lines = [
        "AI Relay / ai-proxy - THIRD-PARTY NOTICES",
        f"Release{root['version']} | Target: {TARGET}",
        f"Root package: {root['name']} {root['version']}",
        f"Third-party crate/version entries: {len(packages)}",
        f"Cargo.toml SHA-256 (LF-normalized UTF-8, BOM removed): {normalized_sha256(manifest)}",
        f"Cargo.lock SHA-256 (LF-normalized UTF-8, BOM removed): {normalized_sha256(lock)}",
        "",
        "SCOPE / 范围",
        "This is a license-notice snapshot of the installed, locked crates.io sources.",
        "Selection: Cargo metadata --offline --locked --format-version 1",
        f"--filter-platform {TARGET}; traverse normal and build edges from the root;",
        "exclude dev-only edges. Default Cargo features are used. Cargo metadata may",
        "unify features with development dependencies; this is a conservative superset,",
        "not proof that every listed crate, feature or source is linked into the EXE.",
        "Build dependencies and procedural macros are included and marked build.",
        "License expressions are Cargo metadata verbatim (legacy '/' denotes OR).",
        "All supplied license alternatives are retained, not relicensed under project MIT.",
        "License texts come from installed crate files, not generated license templates.",
        "Only line endings/BOM are normalized; source excerpts are labeled explicitly.",
        "SHA-256 beside an excerpt identifies its complete original source file.",
        "Identical text is stored once with every applicable crate/source reference.",
        "shlex supplies an abbreviated Apache grant; full Apache-2.0 terms reproduced",
        "from other selected installed crates also accompany that grant below.",
        "The generator uses developer-only Python/Cargo; neither is an application runtime.",
        "本声明是当前依赖快照，不代表已完成法律审核或所有发行验收。",
        "Regenerate and review after dependency/feature/target changes. This is not legal advice.",
        "Toolchain/Windows SDK/CRT redistribution terms are outside this Cargo-crate snapshot",
        "and still require release review; this file is not a complete toolchain license audit.",
        "",
        "BUNDLED COMPONENTS / 捆绑组件",
        "* ring: retain LICENSE, LICENSE-BoringSSL (including its support-code caveats),",
        "  LICENSE-other-bits, vendored once_cell MIT/Apache licenses and Fiat license.",
        "  Implementation/assembly-generator leading legal comments retain per-file",
        "  authorship, BoringSSL/OpenSSL attribution and SUPERCOP public-domain statements.",
        "  Other architectures may be included conservatively; no tests/certificates/keys",
        "  or implementation bodies are copied. Fiat LICENSE refers to an AUTHORS file",
        "  not present in the packaged ring source; its notice is retained verbatim,",
        "  without inventing an author list.",
        "* libsqlite3-sys: MIT applies to the Rust bindings. Bundled SQLite is separate:",
        "  its source's copyright disclaimer/blessing and legal comments follow below.",
        "  SQLite public-domain status is described by those sources, not an invented SPDX",
        "  license. SQLCipher is not enabled; its unrelated source/license is excluded.",
        "* atomic-waker LICENSE-THIRD-PARTY, tracing-core vendored spin MIT notice,",
        "  Unicode data notices and all windows-rs license files are retained.",
        "",
        "CRATE INVENTORY / 依赖清单",
        "crate | version | declared SPDX | reachability | homepage | repository | registry source",
    ]
    texts: dict[str, dict] = {}
    for p in packages:
        source = f"https://crates.io/crates/{p['name']}/{p['version']}"
        lines.append(" | ".join((p["name"], p["version"], p["license"],
                                 "+".join(sorted(roles[p["id"]])), safe_url(p.get("homepage")),
                                 safe_url(p.get("repository")), source)))
        files = license_files(p) + bundled_evidence(p, nodes[p["id"]]["features"])
        for rel, data, text in files:
            # Deduplication never merges different copyright wording.
            normalized = text.rstrip("\n") + "\n"
            digest = sha256(normalized.encode("utf-8"))
            entry = texts.setdefault(digest, {"text": normalized, "references": []})
            entry["references"].append(f"{p['name']} {p['version']} / {rel} / source SHA-256 {sha256(data)}")
    require(any("Version 2.0, January 2004" in entry["text"]
                and "END OF TERMS AND CONDITIONS" in entry["text"] for entry in texts.values()),
            "Missing complete installed Apache-2.0 terms for abbreviated grants")
    lines += ["", "DECLARED LICENSE EXPRESSIONS / 声明许可统计"]
    lines += [f"{expression}: {count}" for expression, count in sorted(counts.items())]
    lines += ["", f"FULL TEXTS AND ATTRIBUTIONS / 许可及署名全文 ({len(texts)} unique texts)"]
    for index, entry in enumerate(texts.values(), 1):
        lines += ["", "=" * 78, f"TEXT {index:03d}", "Applies to / 原始文件："]
        lines += sorted(set(entry["references"]))
        lines += ["-" * 78, entry["text"].rstrip("\n")]
    output = "\n".join(lines) + "\n"
    # Fail, rather than redact and thereby alter a legal text.
    require(not re.search(r"-----BEGIN (?:[A-Z ]*PRIVATE KEY|CERTIFICATE)-----", output),
            "Unexpected private-key/certificate payload in notices")
    require(not re.search(r"(?i)[a-z]:[\\/](?:users|home)[\\/]|/home/|/Users/", output),
            "Unexpected machine-local home path in notices")
    return output, len(packages), counts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cargo", required=True, type=Path, help="Absolute Cargo executable path (developer tool)")
    parser.add_argument("--check", action="store_true", help="Compare generated bytes without writing")
    args = parser.parse_args()
    project = Path(__file__).resolve().parent.parent
    destination = project / "THIRD-PARTY-NOTICES.txt"
    meta, manifest, lock = metadata(args.cargo, project)
    output, count, counts = generate(meta, manifest, lock)
    require(manifest == (project / "Cargo.toml").read_bytes()
            and lock == (project / "Cargo.lock").read_bytes(),
            "Manifest/lock changed during generation; retry after the parent is ready")
    encoded = output.encode("utf-8")
    if args.check:
        require(destination.is_file() and destination.read_bytes() == encoded,
                "THIRD-PARTY-NOTICES.txt is missing or stale; regenerate it")
        print(f"Verified deterministic notices: {count} crate/version entries; {len(encoded)} bytes")
    else:
        destination.write_bytes(encoded)
        print(f"Generated THIRD-PARTY-NOTICES.txt: {count} crate/version entries; {len(encoded)} bytes")
    print("Declared license expressions: " + json.dumps(dict(sorted(counts.items())), ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (NoticeError, OSError, UnicodeError, ValueError, subprocess.SubprocessError) as error:
        print(f"Third-party notices generation failed: {error}", file=sys.stderr)
        sys.exit(1)
