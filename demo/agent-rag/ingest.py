#!/usr/bin/env python3
"""
ingest.py — Load the World Energy Outlook PDF into ChromaDB.

Chunks the PDF, embeds with all-MiniLM-L6-v2, stores in a Chroma
collection called "world_energy_outlook_2025".

Run via docker compose:
  docker compose run --rm \
    -v /path/to/WorldEnergyOutlook2025.pdf:/data/report.pdf:ro \
    ingest

Environment variables (set by docker-compose):
  CHROMA_HOST   — chromadb container hostname (default: chromadb)
  CHROMA_PORT   — chromadb port (default: 8000)
  PDF_PATH      — path to PDF inside container (default: /data/report.pdf)
"""

import os
import sys
import chromadb
import fitz   # PyMuPDF
from sentence_transformers import SentenceTransformer
from tqdm import tqdm

CHROMA_HOST      = os.environ.get("CHROMA_HOST", "chromadb")
CHROMA_PORT      = int(os.environ.get("CHROMA_PORT", "8000"))
PDF_PATH         = os.environ.get("PDF_PATH", "/data/report.pdf")
COLLECTION_NAME  = "world_energy_outlook_2025"
CHUNK_SIZE       = 400   # words
CHUNK_OVERLAP    = 80    # words
SOURCE_NAME      = os.environ.get("SOURCE_NAME", "WorldEnergyOutlook2025")
BATCH_SIZE       = 100


# =============================================================================
# Extract
# =============================================================================
def extract_pages(pdf_path: str) -> list[dict]:
    print(f"\n[1/4] Extracting text from {pdf_path}")
    doc   = fitz.open(pdf_path)
    pages = []
    for i, page in enumerate(doc):
        text = page.get_text("text").strip()
        if text:
            pages.append({"page_num": i + 1, "text": text})
    print(f"      {len(pages)} pages extracted")
    return pages


# =============================================================================
# Chunk
# =============================================================================
def chunk_pages(pages: list[dict]) -> list[dict]:
    print("\n[2/4] Chunking")
    chunks = []
    idx    = 0
    for page in pages:
        words = page["text"].split()
        if len(words) < 20:
            continue
        start = 0
        while start < len(words):
            end  = min(start + CHUNK_SIZE, len(words))
            text = " ".join(words[start:end])
            chunks.append({
                "id":       f"chunk_{idx:05d}",
                "text":     text,
                "source":   f"{SOURCE_NAME} p.{page['page_num']}",
                "page_num": page["page_num"],
                "chunk_idx": idx,
            })
            idx += 1
            if end == len(words):
                break
            start += CHUNK_SIZE - CHUNK_OVERLAP
    print(f"      {len(chunks)} chunks produced")
    return chunks


# =============================================================================
# Embed + store
# =============================================================================
def load_chroma(chunks: list[dict]):
    print(f"\n[3/4] Connecting to Chroma at {CHROMA_HOST}:{CHROMA_PORT}")
    client = chromadb.HttpClient(host=CHROMA_HOST, port=CHROMA_PORT)

    # Get or create collection — always add, never skip
    try:
        col   = client.get_collection(COLLECTION_NAME)
        count = col.count()
        print(f"      Collection '{COLLECTION_NAME}' has {count} existing documents — adding new source")
    except Exception:
        count = 0

    print(f"\n[4/4] Embedding with all-MiniLM-L6-v2 and loading into Chroma")
    model = SentenceTransformer("all-MiniLM-L6-v2")

    col = client.get_or_create_collection(
        name     = COLLECTION_NAME,
        metadata = {"hnsw:space": "cosine"},
    )

    # Process in batches
    for batch_start in tqdm(range(0, len(chunks), BATCH_SIZE), desc="      batches"):
        batch = chunks[batch_start : batch_start + BATCH_SIZE]

        texts      = [c["text"]   for c in batch]
        ids        = [c["id"]     for c in batch]
        metadatas  = [{"source": c["source"], "page_num": c["page_num"], "chunk_idx": c["chunk_idx"]} for c in batch]
        embeddings = model.encode(texts, convert_to_numpy=True).tolist()

        col.add(
            ids        = [f"{SOURCE_NAME}_{i}" for i in ids],
            documents  = texts,
            embeddings = embeddings,
            metadatas  = metadatas,
        )

    final_count = col.count()
    print(f"\n      Collection '{COLLECTION_NAME}': {final_count} documents stored")


# =============================================================================
# Main
# =============================================================================
def main():
    print("=" * 60)
    print("  Ingest — World Energy Outlook 2025 → ChromaDB")
    print(f"  PDF:    {PDF_PATH}")
    print(f"  Chroma: {CHROMA_HOST}:{CHROMA_PORT}")
    print("=" * 60)

    if not os.path.exists(PDF_PATH):
        print(f"\nERROR: PDF not found at {PDF_PATH}")
        print("  Mount it with: -v /your/path/WorldEnergyOutlook2025.pdf:/data/report.pdf:ro")
        sys.exit(1)

    pages  = extract_pages(PDF_PATH)
    chunks = chunk_pages(pages)
    load_chroma(chunks)

    print("\n" + "=" * 60)
    print("  Ingest complete. Agent B is ready to query.")
    print("=" * 60)


if __name__ == "__main__":
    main()