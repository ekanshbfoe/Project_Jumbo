"""The Director's llama.cpp runner without the model: finding the engine, the server command from spike 7e,
and chat requests against a stand-in server."""

import os

import pytest

from jumbo_worker.models.qwen3vl import Qwen3VLRunner, server_binary, server_command
from jumbo_worker.pool.specs import ModelSpec

SPEC = ModelSpec(name="qwen3vl_32b", backend="ggml")


class Response:
    def __init__(self, status_code: int, body: dict | None = None, text: str = "") -> None:
        self.status_code, self.body, self.text = status_code, body, text

    def json(self) -> dict:
        return self.body


class Server:
    def __init__(self, *responses: Response) -> None:
        self.responses = list(responses)
        self.posts = []

    async def post(self, path: str, json: dict) -> Response:
        self.posts.append((path, json))
        return self.responses.pop(0)


def test_the_engine_is_copied_out_of_its_dataset(tmp_path, monkeypatch):
    monkeypatch.delenv("JUMBO_LLAMA_SERVER", raising=False)
    build = tmp_path / "input/jumbo-llamacpp-cuda75/llamacpp-b11925-cuda75"
    build.mkdir(parents=True)
    (build / "llama-server").write_text("#!/bin/sh\n")
    (build / "libggml-cuda.so").write_text("")
    binary = server_binary(str(tmp_path / "input"), str(tmp_path / "bin"))
    assert binary == str(tmp_path / "bin/llama-server")
    assert os.access(binary, os.X_OK) and (tmp_path / "bin/libggml-cuda.so").is_file()


def test_the_server_runs_as_in_spike_7e():
    cmd = server_command("llama-server", "q.gguf", "mm.gguf", 16384, "4")
    joined = " ".join(cmd)
    for part in ("--mmproj mm.gguf", "-ngl 999", "--split-mode layer", "--tensor-split 47,53", "-c 16384",
                 "--flash-attn on", "-ctk q8_0", "-ctv q8_0", "-np 1", "--jinja"):
        assert part in joined


async def test_chat_returns_the_completion_or_the_refusal():
    server = Server(
        Response(200, {"choices": [{"message": {"content": "BEATS:"}, "finish_reason": "stop"}], "usage": {"completion_tokens": 2},
                       "timings": {"predicted_per_second": 6.2}}),
        Response(400, text="unsupported schema: prefixItems"),
    )
    runner = Qwen3VLRunner(SPEC, client=server)
    ok = await runner.run("chat", {"messages": [{"role": "user", "content": "hi"}], "seed": 7})
    assert (ok["status"], ok["content"], ok["finish"], ok["timings"]) == (200, "BEATS:", "stop", {"predicted_per_second": 6.2})
    assert server.posts[0] == ("/v1/chat/completions", {"cache_prompt": True, "messages": [{"role": "user", "content": "hi"}], "seed": 7})
    refused = await runner.run("chat", {"messages": []})
    assert refused["status"] == 400 and "prefixItems" in refused["error"]
    with pytest.raises(ValueError, match="does not serve"):
        await runner.run("speak", {})
