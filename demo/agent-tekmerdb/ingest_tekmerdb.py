#!/usr/bin/env python3
"""
ingest_tekmerdb.py -- Load documents into TekmerDB.

Chunking strategy: sentence-level, not word-count windows.
One meaningful sentence = one PFO. This is what TekmerDB is designed for:
atomic claims that can be individually corroborated or contradicted.

Sentences shorter than MIN_WORDS are skipped (headers, page numbers etc).
Sentences longer than MAX_WORDS are split at natural boundaries (semicolons,
commas) to keep claims atomic.

Runs on the HOST. Requires pfodb running on :3000.

Usage:
    python scripts/ingest_tekmerdb.py --pdf /path/to/WorldEnergyOutlook2025.pdf
    python scripts/ingest_tekmerdb.py --crate <crate_name>
    python scripts/ingest_tekmerdb.py --rust-doc <crate_name_or_url>
    python scripts/ingest_tekmerdb.py --md /path/to/wiki.md --source-name tekmerdb-wiki
"""

import argparse
import os
import pathlib
import re
import sys
import time
import requests
import fitz
from tqdm import tqdm
import json
import bs4
import urllib.parse
import html

TEKMERDB_URL = "http://localhost:3000"
SOURCE_NAME  = "WorldEnergyOutlook2025"  # overridden by --source-name arg
DOMAIN       = "General"
MIN_WORDS    = 8    # skip sentences shorter than this (headers, captions)
MAX_WORDS    = 60   # split sentences longer than this


# =============================================================================
# PDF extraction
# =============================================================================
def extract_pages(pdf_path: str) -> list[dict]:
    print(f"\n[1/3] Extracting text from {pdf_path}")
    doc   = fitz.open(pdf_path)
    pages = []
    for i, page in enumerate(doc):
        text = page.get_text("text").strip()
        if text:
            pages.append({"page_num": i + 1, "text": text})
    print(f"      {len(pages)} pages extracted")
    return pages


# =============================================================================
# Plain text / markdown extraction
# =============================================================================
def extract_text_file(path: str) -> list[dict]:
    """
    Read a plain text or markdown file, splitting into logical pages.

    If the file contains "# ===== WIKI PAGE: <title> =====" markers
    (as produced by refresh_wiki.sh), each marked section becomes its
    own page with the page title carried through as source provenance.
    Otherwise the whole file is treated as a single page.
    """
    print(f"\n[1/3] Reading text file {path}")
    with open(path, "r", encoding="utf-8") as f:
        raw = f.read()

    marker_re = re.compile(r'#\s*=+\s*WIKI PAGE:\s*(.+?)\s*=+\s*\n')
    matches = list(marker_re.finditer(raw))

    if not matches:
        text = raw.strip()
        print(f"      {len(text)} characters read (single page, no wiki markers found)")
        return [{"page_num": 1, "text": text, "page_title": pathlib.Path(path).stem}]

    pages = []
    for i, m in enumerate(matches):
        title = m.group(1).strip()
        start = m.end()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(raw)
        section_text = raw[start:end].strip()
        if section_text:
            pages.append({"page_num": i + 1, "text": section_text, "page_title": title})

    total_chars = sum(len(p["text"]) for p in pages)
    print(f"      {len(pages)} wiki page(s) found, {total_chars} characters total")
    return pages


# =============================================================================
# Sentence-level chunking
# =============================================================================
def clean_text(text: str) -> str:
    """Clean PDF extraction artifacts and wiki page-separator markers."""
    # Strip the "# ===== WIKI PAGE: ... =====" markers inserted by
    # refresh_wiki.sh -- these are structural, not content, and if left
    # in place they get merged into the end of the preceding sentence
    # (the sentence-boundary regex splits on ". " + capital letter, and
    # "#" doesn't trigger a split, so the marker rides along).
    text = re.sub(r'#\s*=+\s*WIKI PAGE:.*?=+\s*', ' ', text)
    # Remove hyphenation at line breaks
    text = re.sub(r'-\n(\w)', r'\1', text)
    # Collapse whitespace and newlines
    text = re.sub(r'\s+', ' ', text)
    return text.strip()


def split_long_sentence(sentence: str, max_words: int) -> list[str]:
    """
    Split a long sentence at natural boundaries if it exceeds max_words.
    Tries semicolons first, then commas before conjunctions.
    """
    words = sentence.split()
    if len(words) <= max_words:
        return [sentence]

    # Try splitting at semicolons
    parts = re.split(r';\s*', sentence)
    if len(parts) > 1:
        result = []
        for part in parts:
            part = part.strip()
            if part and len(part.split()) >= MIN_WORDS:
                result.extend(split_long_sentence(part, max_words))
        if result:
            return result

    # Try splitting at comma + conjunction
    parts = re.split(r',\s+(and|but|while|whereas|however|although|though)\s+', sentence)
    if len(parts) > 1:
        result = []
        for part in parts:
            part = part.strip()
            if part and len(part.split()) >= MIN_WORDS:
                result.extend(split_long_sentence(part, max_words))
        if result:
            return result

    # Can't split cleanly — keep as is
    return [sentence]


def extract_sentences(pages: list[dict]) -> list[dict]:
    """
    Extract individual sentences from pages.
    Returns list of {sentence, source_label, page_num}
    """
    print("\n[2/3] Extracting sentences")

    # Patterns that indicate non-content lines to skip
    skip_patterns = [
        r'^\d+$',                          # page numbers
        r'^Figure \d+',                    # figure captions start
        r'^Table \d+',                     # table captions
        r'^Chapter \d+',                   # chapter headers
        r'^International Energy Agency',   # running headers
        r'^World Energy Outlook',          # running headers
        r'^IEA',                           # running headers
        r'^\s*[\|\-\+]{3,}',              # table borders
        r'^\s*\d+[\s\|]',                 # table row starts
        r'^#+\s',                          # markdown headers (handled separately below)
    ]
    skip_re = [re.compile(p, re.IGNORECASE) for p in skip_patterns]

    sentences = []
    total_skipped = 0

    for page in pages:
        text  = clean_text(page["text"])
        if page.get("page_title"):
            label = f"{SOURCE_NAME}/{page['page_title']}"
        else:
            label = f"{SOURCE_NAME} p.{page['page_num']}"

        # Split into sentences at ". " "! " "? " followed by capital letter
        raw_sentences = re.split(r'(?<=[.!?])\s+(?=[A-Z])', text)

        for sent in raw_sentences:
            sent = sent.strip()
            if not sent:
                continue

            # Skip obvious non-content
            if any(p.match(sent) for p in skip_re):
                total_skipped += 1
                continue

            # Skip too-short sentences
            word_count = len(sent.split())
            if word_count < MIN_WORDS:
                total_skipped += 1
                continue

            # Skip sentences that are mostly numbers/symbols (tables)
            alpha_ratio = sum(c.isalpha() for c in sent) / max(len(sent), 1)
            if alpha_ratio < 0.4:
                total_skipped += 1
                continue

            # Split long sentences into atomic claims
            parts = split_long_sentence(sent, MAX_WORDS)
            for part in parts:
                part = part.strip()
                if len(part.split()) >= MIN_WORDS:
                    sentences.append({
                        "sentence":     part,
                        "source_label": label,
                        "page_num":     page["page_num"],
                    })

    print(f"      {len(sentences)} sentences extracted  ({total_skipped} skipped)")
    return sentences


# =============================================================================
# TekmerDB load
# =============================================================================
def wait_for_engine(url: str) -> int:
    print(f"\n  Waiting for TekmerDB at {url}...")
    for _ in range(20):
        try:
            r = requests.get(f"{url}/health", timeout=3)
            if r.status_code == 200:
                count = r.json().get("pfo_count", 0)
                print(f"  Engine ready -- {count} PFOs currently stored")
                return count
        except Exception:
            pass
        time.sleep(2)
    print("  ERROR: TekmerDB not responding. Is pfodb running on :3000?")
    sys.exit(1)


def load_tekmerdb(sentences: list[dict], url: str):
    print(f"\n[3/3] Loading into TekmerDB ({url})")
    existing = wait_for_engine(url)

    if existing > 0:
        print(f"  Engine has {existing} existing PFOs -- adding new source on top")

    inserted = skipped = domain_rejected = 0

    for item in tqdm(sentences, desc="  inserting"):
        payload = {
            "claim_text": item["sentence"],
            "confidence": 0.8,
            "source":     item["source_label"],
            "domain":     DOMAIN,
        }
        try:
            r = requests.post(f"{url}/pfo", json=payload, timeout=15)
            if r.status_code == 200:
                data   = r.json()
                status = data.get("status", "")
                if status == "inserted":
                    inserted += 1
                elif status == "duplicate":
                    skipped += 1
                else:
                    skipped += 1
                pass  # conflict detection is async — not tracked at insert time
            elif r.status_code == 422:
                domain_rejected += 1
            else:
                skipped += 1
                tqdm.write(f"  WARN {r.status_code}: {r.text[:60]}")
        except Exception as e:
            skipped += 1
            tqdm.write(f"  ERROR: {e}")

    r     = requests.get(f"{url}/health", timeout=5)
    final = r.json().get("pfo_count", 0)
    print(f"\n  inserted:        {inserted}")
    print(f"  domain rejected: {domain_rejected}")
    print(f"  duplicates:      {skipped}")
    print(f"  TekmerDB final PFO count: {final}")


# =============================================================================
# Crate mode
# =============================================================================
def fetch_crate_info(crate_name: str) -> dict:
    """Fetch crate info from crates.io API.

    Note: must use /api/v1/crates/{name}, not /api/crates/{name} -- the
    latter 404s. Response is nested under a top-level "crate" key.
    """
    url = f"https://crates.io/api/v1/crates/{crate_name}"
    response = requests.get(url, headers={"User-Agent": "tekmerdb-ingest/1.0"})
    response.raise_for_status()
    data = response.json()
    return data["crate"]


def extract_crate_sentences(crate_info: dict) -> list[dict]:
    """Extract sentences from crate info.

    Field names match the real crates.io /api/v1/crates/{name} response:
    - version: use max_stable_version (falls back to newest_version)
    - categories/keywords: flat lists of strings, not dicts
    - license, dependencies: NOT present at this endpoint (that's a
      per-version endpoint) -- omitted rather than guessed.
    """
    sentences = []
    crate_name = crate_info.get("name", "")
    version = crate_info.get("max_stable_version") or crate_info.get("newest_version", "")
    description = (crate_info.get("description") or "").strip()
    categories = crate_info.get("categories", [])
    keywords = crate_info.get("keywords", [])
    repository = crate_info.get("repository", "")
    homepage = crate_info.get("homepage", "")

    # Extract sentence from description
    if description:
        sentences.append({
            "sentence": f"The crate '{crate_name}' version {version} is described as: {description}",
            "source_label": f"crates.io/{crate_name}",
            "page_num": 1,
        })

    # Extract sentences from categories (flat list of strings)
    for category in categories:
        sentences.append({
            "sentence": f"The crate '{crate_name}' belongs to the category '{category}'.",
            "source_label": f"crates.io/{crate_name}",
            "page_num": 1,
        })

    # Extract sentences from keywords (flat list of strings)
    for keyword in keywords:
        sentences.append({
            "sentence": f"The crate '{crate_name}' has the keyword '{keyword}'.",
            "source_label": f"crates.io/{crate_name}",
            "page_num": 1,
        })

    # Version fact -- the single most useful claim for avoiding the
    # "what version should I pin" hallucination loop.
    if version:
        sentences.append({
            "sentence": f"The latest stable version of the crate '{crate_name}' is {version}.",
            "source_label": f"crates.io/{crate_name}",
            "page_num": 1,
        })

    if repository:
        sentences.append({
            "sentence": f"The crate '{crate_name}' source repository is {repository}.",
            "source_label": f"crates.io/{crate_name}",
            "page_num": 1,
        })

    return sentences


# =============================================================================
# Rust-doc mode
# =============================================================================
def fetch_rust_doc(crate_name_or_url: str) -> str:
    """Fetch HTML pages from doc.rust-lang.org or docs.rs."""
    if crate_name_or_url.startswith("http"):
        url = crate_name_or_url
    else:
        url = f"https://docs.rs/{crate_name_or_url}/latest/"
    response = requests.get(url)
    response.raise_for_status()
    return response.text


def extract_rust_doc_sentences(html_content: str) -> list[dict]:
    """Extract sentences from HTML content."""
    soup = bs4.BeautifulSoup(html_content, 'html.parser')
    text = soup.get_text()
    sentences = extract_sentences([{"page_num": 1, "text": text}])
    return sentences


# =============================================================================
# Main
# =============================================================================
def main():
    global SOURCE_NAME
    parser = argparse.ArgumentParser(
        description="Ingest documents into TekmerDB -- sentence-level chunking"
    )
    parser.add_argument("--pdf", required=False, help="Path to PDF file")
    parser.add_argument("--crate", required=False, help="Crate name to fetch from crates.io")
    parser.add_argument("--rust-doc", required=False, help="Crate name or URL to fetch documentation from")
    parser.add_argument("--md", required=False, help="Path to a local plain text or markdown file")
    parser.add_argument("--url", default=TEKMERDB_URL, help="TekmerDB base URL")
    parser.add_argument("--source-name", default=None, help="Source name prefix (default: auto from filename)")
    args = parser.parse_args()

    modes_given = [bool(args.pdf), bool(args.crate), bool(args.rust_doc), bool(args.md)]
    if sum(modes_given) == 0:
        parser.error("One of --pdf, --crate, --rust-doc, or --md must be specified")
    if sum(modes_given) > 1:
        parser.error("Only one of --pdf, --crate, --rust-doc, or --md can be specified")

    if args.pdf:
        if not os.path.exists(args.pdf):
            print(f"\nERROR: PDF not found: {args.pdf}")
            sys.exit(1)
        SOURCE_NAME = args.source_name or pathlib.Path(args.pdf).stem
        pages = extract_pages(args.pdf)
        sentences = extract_sentences(pages)
        load_tekmerdb(sentences, args.url)

    if args.crate:
        SOURCE_NAME = args.source_name or f"crates.io-{args.crate}"
        crate_info = fetch_crate_info(args.crate)
        sentences = extract_crate_sentences(crate_info)
        load_tekmerdb(sentences, args.url)

    if args.rust_doc:
        SOURCE_NAME = args.source_name or args.rust_doc
        html_content = fetch_rust_doc(args.rust_doc)
        sentences = extract_rust_doc_sentences(html_content)
        load_tekmerdb(sentences, args.url)

    if args.md:
        if not os.path.exists(args.md):
            print(f"\nERROR: file not found: {args.md}")
            sys.exit(1)
        SOURCE_NAME = args.source_name or pathlib.Path(args.md).stem
        pages = extract_text_file(args.md)
        sentences = extract_sentences(pages)
        load_tekmerdb(sentences, args.url)

    print("\n" + "=" * 60)
    print("  Done. Agent A (TekmerDB) is ready.")
    print("=" * 60)


if __name__ == "__main__":
    main()