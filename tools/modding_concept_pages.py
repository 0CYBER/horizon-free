#!/usr/bin/env python3
"""mdBook preprocessor: build the modding book's chapter tree from its folders, so a new page needs
no edit to SUMMARY.md, and check every page's links.

- A page `X.md` with a folder `X/` beside it gets the folder's pages as its sub-chapters, in file
  name order, each titled by its first heading (and so on, one folder down).
  `<!-- section-pages -->` in such a page becomes a list of links to those sub-chapters.
- A page directly in one of FOLDERS that SUMMARY.md doesn't list joins the end of its part, by title.
- Every relative link, reference destination and `{{#include}}` must reach a file (or a generated
  chapter) and, for a page, an existing heading anchor; root-relative links are refused.
- A page the book doesn't render, or one titled as a design, a plan or a work list, fails the build.
"""
import argparse
from collections import Counter
from html.parser import HTMLParser
from itertools import chain
import json
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "docs/modding/src"
LINK = re.compile(r"(\!?\[[^\n]*?\]\()([^\s)]+)(\))")
REFERENCE = re.compile(r"^( {0,3}\[[^\]\n]+\]:[ \t]*<?)([^\s>]+)(>?)", re.MULTILINE)
INCLUDE = re.compile(r"(\{\{#(?:rustdoc_)?include )([^ }]+)(\s*\}\})")
# Each page directly in one of these folders is a chapter of the part: the pages SUMMARY.md lists keep
# their place, the rest follow in title order.
FOLDERS = {"Concepts": "concepts", "How-to": "how-to"}
SECTION_PAGES = "<!-- section-pages -->"
# The book documents what the game does. A page titled as one of these is a design, a plan or a work list,
# and lives in docs/design/ instead.
NOT_A_MANUAL = re.compile(r"\b(design|designs|proposal|proposed|draft|roadmap|tracker|status|plan)\b", re.IGNORECASE)


def headings(text):
    """Yield ATX headings outside fenced examples, using mdBook's heading IDs."""
    fenced = False
    counts = Counter()
    for line in text.splitlines():
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        if not fenced and re.match(r"^#{1,6} ", line):
            title = line.split(" ", 1)[1]
            explicit = re.fullmatch(r"(.*?)\s*\{#([\w-]+)\}\s*", title)
            if explicit:
                yield explicit[1], explicit[2]
                continue
            slug = re.sub(r"[^\w\- ]", "", title.lower()).replace(" ", "-")
            n = counts[slug]
            counts[slug] += 1
            yield title, slug + (f"-{n}" if n else "")


def page_title(text, path):
    return next((title for title, _ in headings(text)), path.stem.replace("-", " ").capitalize())


def prose(text):
    """`text` without fenced code blocks and inline code spans, whose brackets are not links."""
    out, fenced = [], False
    for line in text.splitlines():
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            continue
        if not fenced:
            out.append(re.sub(r"(`+)(?:(?!\1).)+?\1", "", line))
    return "\n".join(out)


def validate(text, path, src, contents):
    body = prose(text)
    for match in chain(LINK.finditer(body), REFERENCE.finditer(body)):
        url = urlsplit(match[2])
        if url.scheme or url.netloc:
            continue
        assert not unquote(url.path).startswith("/"), f"{path}: root-relative link {match[2]}"
        target = (src / path).parent / unquote(url.path) if url.path else src / path
        target = target.resolve()
        assert target in contents or target.is_file(), f"{path}: missing link target {match[2]}"
        if url.fragment and target.suffix == ".md":
            body = contents[target] if target in contents else target.read_text()
            assert unquote(url.fragment) in {a for _, a in headings(body)}, f"{path}: missing anchor {match[2]}"
    for match in INCLUDE.finditer(text):
        name, _, anchor = match[2].partition(":")
        target = (src / path).parent / name
        assert target.is_file(), f"{path}: missing include {name}"
        if anchor and not anchor[0].isdigit():
            assert f"ANCHOR: {anchor}" in target.read_text(), f"{path}: missing include anchor {anchor}"


def chapter(src, relative, number, parents):
    text = (src / relative).read_text()
    title = page_title(text, src / relative)
    return dict(name=title.replace("`", ""), content=text, path=relative, source_path=relative,
                number=number, parent_names=parents, sub_items=[])


def nest(chapter_, src):
    """Gives a chapter whose page has a folder beside it that folder's pages, in file name order."""
    folder = (src / chapter_["path"]).with_suffix("")
    if not folder.is_dir():
        return
    listed = {item["Chapter"]["path"] for item in chapter_["sub_items"] if "Chapter" in item}
    parents = chapter_["parent_names"] + [chapter_["name"]]
    for path in sorted(folder.glob("*.md")):
        relative = path.relative_to(src).as_posix()
        if relative not in listed:
            chapter_["sub_items"].append({"Chapter": chapter(src, relative, [], parents)})
    for item in chapter_["sub_items"]:
        if "Chapter" in item and item["Chapter"]["path"]:
            nest(item["Chapter"], src)
    here = Path(chapter_["path"]).parent
    links = "\n".join(f"- [{c['name']}]({Path(c['path']).relative_to(here).as_posix()})"
                      for c in (i["Chapter"] for i in chapter_["sub_items"] if "Chapter" in i))
    chapter_["content"] = chapter_["content"].replace(SECTION_PAGES, links)


def chapters(items):
    """Every chapter of the book, depth first."""
    for item in items:
        if "Chapter" in item:
            yield item["Chapter"]
            yield from chapters(item["Chapter"]["sub_items"])


def discover(book, src):
    """Adds each page of FOLDERS that SUMMARY.md doesn't list at the end of its part, by title."""
    items = book["sections"]
    listed = {c["path"] for c in chapters(items) if c["path"]}
    for part, folder in FOLDERS.items():
        start = next((i for i, item in enumerate(items) if item.get("PartTitle") == part), None)
        assert start is not None, f"SUMMARY must have a # {part} part"
        end = next((i for i in range(start + 1, len(items)) if "PartTitle" in items[i]), len(items))
        found = [chapter(src, p.relative_to(src).as_posix(), [], [])
                 for p in (src / folder).glob("*.md") if p.relative_to(src).as_posix() not in listed]
        found.sort(key=lambda c: (c["name"].casefold(), c["path"]))
        items[end:end] = [{"Chapter": c} for c in found]


def renumber(items, prefix):
    """Numbers the chapters again as mdBook does: in order, across parts."""
    index = 0
    for item in items:
        chapter_ = item.get("Chapter")
        if chapter_ is None or chapter_.get("number") is None:
            continue
        index += 1
        chapter_["number"] = prefix + [index]
        renumber(chapter_["sub_items"], chapter_["number"])


def unrendered(book, src):
    """The Markdown sources under `src` that no chapter of `book` renders."""
    rendered = {c["path"] for c in chapters(book["sections"]) if c["path"]}
    return sorted(path.relative_to(src).as_posix() for path in src.rglob("*.md")
                  if path.name != "SUMMARY.md" and path.relative_to(src).as_posix() not in rendered)


def refuse_designs(book):
    """Fails on a chapter whose name or first heading reads as a design or a work list."""
    for chapter_ in chapters(book["sections"]):
        if not chapter_["path"]:
            continue
        titles = [chapter_["name"], *(title for title, _ in headings(chapter_["content"]))][:2]
        for title in titles:
            assert not NOT_A_MANUAL.search(title), (
                f"{chapter_['path']}: \"{title}\" reads as a design or a work list; the book documents what "
                "the game does, and designs, plans and trackers live in docs/design/")


def expand(book, src):
    discover(book, src)
    for chapter_ in list(chapters(book["sections"])):
        if chapter_["path"] and (src / chapter_["path"]).is_file():
            nest(chapter_, src)
    renumber(book["sections"], [])
    contents = {(src / c["path"]).resolve(): c["content"] for c in chapters(book["sections"]) if c["path"]}
    for chapter_ in chapters(book["sections"]):
        if chapter_["path"] and chapter_.get("source_path"):
            validate(chapter_["content"], chapter_["path"], src, contents)
    missing = unrendered(book, src)
    assert not missing, f"pages the book doesn't render (list them in SUMMARY.md or put them in a folder): {missing}"
    refuse_designs(book)
    return book


class HtmlLinks(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.ids, self.links = set(), []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        if tag == "a" and "href" in attrs:
            self.links.append(attrs["href"])


def check_html(directory, src=SRC):
    pages = sorted(p.relative_to(src).as_posix() for p in src.rglob("*.md") if p.name != "SUMMARY.md")
    # The generated chapters have no source page.
    pages += [p for p in ("reference/hook-notes.md", "reference/neoforge-counterparts.md")
              if (directory / p).with_suffix(".html").is_file()]
    for page in pages:
        path = directory / Path(page).with_suffix(".html")
        html = HtmlLinks(path.read_text())
        for link in html.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            assert not unquote(url.path).startswith("/"), f"{page}: root-relative link {link}"
            target = (path.parent / unquote(url.path)).resolve() if url.path else path
            assert target.is_relative_to(directory.resolve()) and target.is_file(), (
                f"{page}: missing rendered link {link} (link to repository files with a GitHub URL)")
            if url.fragment:
                assert unquote(url.fragment) in HtmlLinks(target.read_text()).ids, f"{page}: missing rendered anchor {link}"
    print(f"Rendered relative links and anchors: valid in {len(pages)} chapters")


def main():
    if sys.argv[1:2] == ["supports"]:
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--html", type=Path)
    args = parser.parse_args()
    if args.html:
        check_html(args.html.resolve())
    else:
        context, book = json.load(sys.stdin)
        print(json.dumps(expand(book, Path(context["root"]) / context["config"]["book"]["src"])))


if __name__ == "__main__":
    main()
