#!/usr/bin/env python3
"""Experiment: import a reflowable EPUB 3 using Pandoc and ibisOffice."""
import argparse
import html
import json
import re
import subprocess
import tempfile
import zipfile
from html.parser import HTMLParser
from pathlib import Path, PurePosixPath
from urllib.parse import quote, unquote
from xml.etree import ElementTree as ET

EPUB_TYPE = "{http://www.idpf.org/2007/ops}type"
RUBY = re.compile(r"<ruby(?:\s[^>]*)?>([^<]+)<rt(?:\s[^>]*)?>([^<]+)</rt></ruby>")


def outline(book):
    container = ET.fromstring(book.read("META-INF/container.xml"))
    package_path = container.find(".//{*}rootfile").attrib["full-path"]
    package = ET.fromstring(book.read(package_path))
    if package.attrib.get("version", "").split(".")[0] != "3":
        raise ValueError("This probe requires EPUB 3")
    if any(node.get("property") == "rendition:layout" and (node.text or "").strip() == "pre-paginated"
           for node in package.findall(".//{*}meta")):
        raise ValueError("This probe requires reflowable EPUB")
    manifest = {entry.attrib["id"]: entry for entry in package.findall("./{*}manifest/{*}item")}
    spine = package.findall("./{*}spine/{*}itemref")
    if any("rendition:layout-pre-paginated" in item.get("properties", "").split() for item in spine):
        raise ValueError("This probe requires a reflowable spine")
    nav = next(item for item in manifest.values() if "nav" in item.get("properties", "").split())
    nav_path = PurePosixPath(package_path).parent / unquote(nav.attrib["href"])
    nav_xml = ET.fromstring(book.read(str(nav_path)))
    toc = next(node for node in nav_xml.iterfind(".//{*}nav") if "toc" in node.get(EPUB_TYPE, "").split())

    def entries(parent):
        if parent is None:
            return []
        result = []
        for item in parent.findall("./{*}li"):
            label = next((el for el in item if el.tag.rsplit("}", 1)[-1] in ("a", "span")), None)
            if label is None:
                continue
            result.append({"label": " ".join("".join(label.itertext()).split()),
                           "href": label.get("href"), "children": entries(item.find("./{*}ol"))})
        return result

    return {
        "navigation": str(nav_path),
        "spine": [manifest[item.attrib["idref"]].attrib["href"] for item in spine],
        "toc": entries(toc.find("./{*}ol")),
    }


class ImageAttributes(HTMLParser):
    def handle_starttag(self, tag, attrs):
        self.attrs = dict(attrs)


def markdown_images(markdown):
    def replace(match):
        parser = ImageAttributes()
        parser.feed(match[0])
        src = parser.attrs.get("src") or ""
        if not src.startswith("assets/") or ".." in PurePosixPath(unquote(src)).parts:
            return match[0]
        alt = parser.attrs.get("alt") or ""
        alt = re.sub(r"([\\\[\]])", r"\\\1", alt)
        return f"![{alt}]({quote(src, safe='/%')})"

    # Pandoc fence delimiters are kept verbatim, including their examples.
    fence = None
    result = []
    for line in markdown.splitlines(keepends=True):
        delimiter = re.match(r"^ {0,3}(`{3,}|~{3,})", line)
        if delimiter:
            marker = delimiter[1]
            if fence is None:
                fence = marker
            elif marker.startswith(fence) and not line[delimiter.end():].strip():
                fence = None
            result.append(line)
        else:
            result.append(line if fence else re.sub(r"(?<!\\)<img\b[^>]*>", replace, line))
    return "".join(result)


def convert(source, output, ibis):
    output.mkdir(parents=True, exist_ok=True)
    book_path = output / "book.md"
    rubies = {}

    def mark(match):
        key = f"IBISEPUBRUBY{len(rubies):08d}TAG"
        base, reading = (html.unescape(part) for part in match.groups())
        if any(ch in base + reading for ch in "|{}\\\n"):
            return match[0]
        rubies[key] = f"{{{base}|{reading}}}"
        return key

    with tempfile.TemporaryDirectory() as tmp, zipfile.ZipFile(source) as book:
        info = outline(book)
        staged = Path(tmp) / "book.epub"
        with zipfile.ZipFile(staged, "w", compression=zipfile.ZIP_DEFLATED) as dest:
            dest.writestr("mimetype", book.read("mimetype"), compress_type=zipfile.ZIP_STORED)
            for name in book.namelist():
                if name == "mimetype" or name.endswith("/"):
                    continue
                data = book.read(name)
                if name.endswith((".html", ".xhtml")):
                    data = RUBY.sub(mark, data.decode("utf-8")).encode()
                dest.writestr(name, data)
        subprocess.run(["pandoc", "-f", "epub", "-t", "gfm+footnotes+yaml_metadata_block",
                        "--wrap=none", "-s", "--extract-media=assets", str(staged), "-o", "book.md"],
                       cwd=output, check=True)
    markdown = book_path.read_text()
    markdown = re.sub(r"IBISEPUBRUBY\d{8}TAG", lambda match: rubies.get(match[0], match[0]), markdown)
    book_path.write_text(markdown_images(markdown))
    (output / "epub-structure.yaml").write_text(json.dumps(info, ensure_ascii=False, indent=2) + "\n")
    subprocess.run([str(ibis), "--export", "docx", "--out-dir", str(output), str(book_path)], check=True)
    print(f"{source.name}: {len(info['spine'])} spine entries, {len(info['toc'])} top-level TOC entries, {len(rubies)} simple rubies")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("epub", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--ibis-office", type=Path, required=True)
    args = parser.parse_args()
    convert(args.epub.resolve(), args.output.resolve(), args.ibis_office.resolve())


if __name__ == "__main__":
    main()
