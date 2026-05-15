"""
server.py — FastAPI HTTP wrapper around agent_rag.

Endpoints:
  GET  /health          liveness + readiness check
  POST /ask             {"question": "..."} → {"answer": ..., "tool_calls": ..., ...}

compare.py on the host POSTs to http://localhost:8080/ask to drive Agent B.
"""

import asyncio
import logging
import os

import uvicorn
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

from agent_rag import RagAgent

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(name)s %(levelname)s %(message)s",
)
log = logging.getLogger("server")

app   = FastAPI(title="Energy Compliance Agent — RAG")
agent = RagAgent()


class AskRequest(BaseModel):
    question: str


class AskResponse(BaseModel):
    answer:     str
    tool_calls: list[dict]
    elapsed_s:  float
    backend:    str


@app.on_event("startup")
async def startup():
    log.info("Starting RAG agent...")
    await agent.start()
    log.info("RAG agent ready")


@app.on_event("shutdown")
async def shutdown():
    await agent.stop()


@app.get("/health")
async def health():
    return {
        "status":  "ready" if agent._ready else "starting",
        "backend": "rag-chroma",
        "ollama":  os.environ.get("OLLAMA_URL"),
        "chroma":  f"{os.environ.get('CHROMA_HOST')}:{os.environ.get('CHROMA_PORT')}",
    }


@app.post("/ask", response_model=AskResponse)
async def ask(req: AskRequest):
    if not req.question.strip():
        raise HTTPException(status_code=400, detail="question cannot be empty")
    if not agent._ready:
        raise HTTPException(status_code=503, detail="agent not ready yet")

    result = await agent.ask(req.question)

    if "error" in result:
        raise HTTPException(status_code=500, detail=result["error"])

    return AskResponse(**result)


if __name__ == "__main__":
    port = int(os.environ.get("AGENT_PORT", 8080))
    uvicorn.run("server:app", host="0.0.0.0", port=port, log_level="info")
