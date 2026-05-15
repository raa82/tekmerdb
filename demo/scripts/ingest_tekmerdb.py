#!/usr/bin/env python3
"""
ingest_tekmerdb.py -- Load the World Energy Outlook PDF into TekmerDB.

Chunking strategy: sentence-level, not word-count windows.
One meaningful sentence = one PFO. This is what TekmerDB is designed for:
atomic claims that can be individually corroborated or contradicted.

Sentences shorter than MIN_WORDS are skipped (headers, page numbers etc).
Sentences longer than MAX_WORDS are split at natural boundaries (semicolons,
commas) to keep claims atomic.

Runs on the HOST. Requires pfodb running on :3000.

Usage:
    python scripts/ingest_tekmerdb.py --pdf /path/to/WorldEnergyOutlook2025.pdf
"""

import argparse
import os
import re
import sys
import time
import requests
import fitz
from tqdm import tqdm

TEKMERDB_URL = "http://localhost:3000"
SOURCE_NAME  = "WorldEnergyOutlook2025"  # overridden by --source-name arg
DOMAIN       = "CriticalInfrastructure"
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
# Sentence-level chunking
# =============================================================================
def clean_text(text: str) -> str:
    """Clean PDF extraction artifacts."""
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
    ]
    skip_re = [re.compile(p, re.IGNORECASE) for p in skip_patterns]

    sentences = []
    total_skipped = 0

    for page in pages:
        text  = clean_text(page["text"])
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
    print(f"  conflicts flagged during sweep: {conflicts}")
    print(f"  TekmerDB final PFO count: {final}")


# =============================================================================
# Main
# =============================================================================
def main():
    global SOURCE_NAME
    parser = argparse.ArgumentParser(
        description="Ingest PDF into TekmerDB -- sentence-level chunking"
    )
    parser.add_argument("--pdf", required=True, help="Path to PDF file")
    parser.add_argument("--url", default=TEKMERDB_URL, help="TekmerDB base URL")
    parser.add_argument("--source-name", default=None, help="Source name prefix (default: auto from filename)")
    args = parser.parse_args()

    print("=" * 60)
    print("  TekmerDB Ingest -- World Energy Outlook 2025")
    print(f"  PDF    : {args.pdf}")
    print(f"  Engine : {args.url}")
    print(f"  Source : {SOURCE_NAME}")
    print(f"  Mode   : sentence-level (one claim per PFO)")
    print("=" * 60)

    if not os.path.exists(args.pdf):
        print(f"\nERROR: PDF not found: {args.pdf}")
        sys.exit(1)

    if args.source_name:
        SOURCE_NAME = args.source_name
    else:
        # Auto-derive from filename
        import pathlib
        SOURCE_NAME = pathlib.Path(args.pdf).stem

    pages     = extract_pages(args.pdf)
    sentences = extract_sentences(pages)
    load_tekmerdb(sentences, args.url)

    print("\n" + "=" * 60)
    print("  Done. Agent A (TekmerDB) is ready.")
    print("=" * 60)


if __name__ == "__main__":
    main()