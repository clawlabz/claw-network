"""Tests for rpc module — JSON-RPC 2.0 client."""

import json
import pytest
import httpx
import respx
from clawminer.rpc import rpc_call, RpcError


def test_rpc_call_format():
    """RPC call should produce correct JSON-RPC 2.0 envelope."""
    with respx.mock:
        route = respx.post("http://localhost:9070/").mock(
            return_value=httpx.Response(
                200,
                json={"jsonrpc": "2.0", "id": 1, "result": "0x42"},
            )
        )

        result = rpc_call("http://localhost:9070/", "claw_getBalance", ["0xabc"])
        assert result == "0x42"

        # Verify the request body
        request = route.calls[0].request
        body = json.loads(request.content)
        assert body["jsonrpc"] == "2.0"
        assert body["method"] == "claw_getBalance"
        assert body["params"] == ["0xabc"]
        assert "id" in body


def test_rpc_call_no_params():
    """RPC call with no params should send empty list."""
    with respx.mock:
        route = respx.post("http://localhost:9070/").mock(
            return_value=httpx.Response(
                200,
                json={"jsonrpc": "2.0", "id": 1, "result": 42},
            )
        )

        result = rpc_call("http://localhost:9070/", "claw_blockNumber")
        assert result == 42

        body = json.loads(route.calls[0].request.content)
        assert body["params"] == []


def test_error_handling_rpc_error():
    """JSON-RPC error response should raise RpcError."""
    with respx.mock:
        respx.post("http://localhost:9070/").mock(
            return_value=httpx.Response(
                200,
                json={
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": {"code": -32600, "message": "Invalid request"},
                },
            )
        )

        with pytest.raises(RpcError, match="Invalid request"):
            rpc_call("http://localhost:9070/", "bad_method")


def test_error_handling_network():
    """Network errors should raise meaningful exceptions."""
    with respx.mock:
        respx.post("http://unreachable:9070/").mock(
            side_effect=httpx.ConnectError("Connection refused")
        )

        with pytest.raises(ConnectionError):
            rpc_call("http://unreachable:9070/", "claw_blockNumber")
